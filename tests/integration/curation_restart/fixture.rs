use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use nexofolio_backend::wiring::{Config, Databases};
use serde_json::{Value, json};
use std::{
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpListener,
    process::{Child, Command},
    sync::Notify,
    task::JoinHandle,
};

pub fn database_url() -> String {
    let url = std::env::var("TEST_DATABASE_URL").expect("isolated database required");
    let parsed = reqwest::Url::parse(&url).expect("valid test database URL required");
    assert!(
        parsed
            .path()
            .trim_start_matches('/')
            .starts_with("nexofolio_recovery_"),
        "recovery process tests require an isolated nexofolio_recovery_* database"
    );
    url
}

#[derive(Default)]
pub struct Model {
    pub block: AtomicBool,
    pub waiting: AtomicBool,
    pub release: Notify,
    pub paths: Mutex<Vec<Vec<String>>>,
}
async fn complete(State(s): State<Arc<Model>>, Json(body): Json<Value>) -> Json<Value> {
    let prompt = body["messages"][0]["content"].as_str().unwrap();
    let content = if let Some(raw) = prompt.split("字段和可核查的证据：").nth(1) {
        let inputs: Vec<Value> = serde_json::from_str(raw.lines().next().unwrap()).unwrap();
        let paths: Vec<String> = inputs
            .iter()
            .map(|i| i["path"].as_str().unwrap().to_owned())
            .collect();
        s.paths.lock().unwrap().push(paths.clone());
        if paths == ["f2"] && s.block.load(Ordering::SeqCst) {
            s.waiting.store(true, Ordering::SeqCst);
            s.release.notified().await;
        }
        json!({"fields":inputs.iter().filter(|i|i["path"]!="f2" || !s.block.load(Ordering::SeqCst)).map(|i|json!({"key":i["key"],"display_name":"合成字段","description":"已核对的合成业务说明"})).collect::<Vec<_>>()})
    } else {
        json!({"name":"重启续跑合成接口","purpose":"查询合成业务资料"})
    };
    Json(json!({"choices":[{"message":{"content":content.to_string()},"finish_reason":"stop"}]}))
}
async fn token() -> Json<Value> {
    Json(json!({"token":"fixture-upstream-token"}))
}
async fn user() -> Json<Value> {
    Json(
        json!({"profile":{"id":1,"account":"operator","realname":"测试发起人","deleted":"0","view":{"projects":"11"}}}),
    )
}
async fn projects() -> Json<Value> {
    Json(
        json!({"page":1,"limit":100,"total":1,"projects":[{"id":11,"name":"重启测试项目","status":"doing"}]}),
    )
}

pub struct Upstream {
    pub base: String,
    pub model: Arc<Model>,
    task: JoinHandle<()>,
}
impl Upstream {
    pub async fn start() -> Self {
        let model = Arc::new(Model::default());
        model.block.store(true, Ordering::SeqCst);
        let app = Router::new()
            .route("/api.php/v1/tokens", post(token))
            .route("/api.php/v1/user", get(user))
            .route("/api.php/v1/projects", get(projects))
            .route("/v1/chat/completions", post(complete))
            .with_state(model.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { base, model, task }
    }
    pub fn config(&self, db: &str) -> Config {
        Config::from_lookup(|name| match name {
            "DATABASE_URL" => Some(db.into()),
            "NEXOFOLIO_ZENTAO_BASE_URL" => Some(format!("{}/api.php/v1", self.base)),
            "NEXOFOLIO_SESSION_KEY" => Some("ab".repeat(32)),
            _ => None,
        })
        .unwrap()
    }
    pub async fn databases(&self, db: &str) -> Databases {
        let databases = Databases::new(&self.config(db)).unwrap();
        databases.migrate().await.unwrap();
        databases
    }
}
impl Drop for Upstream {
    fn drop(&mut self) {
        self.model.release.notify_waiters();
        self.task.abort();
    }
}

pub struct ApiProcess {
    pub url: String,
    pub child: Child,
    drain: JoinHandle<()>,
}
impl ApiProcess {
    pub async fn start(db: &str, upstream: &Upstream) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_nexofolio-api"))
            .env_clear()
            .env("DATABASE_URL", db)
            .env("NEXOFOLIO_BIND_ADDR", "127.0.0.1:0")
            .env("NEXOFOLIO_LOG", "info")
            .env(
                "NEXOFOLIO_ZENTAO_BASE_URL",
                format!("{}/api.php/v1", upstream.base),
            )
            .env("NEXOFOLIO_SESSION_KEY", "ab".repeat(32))
            .env(
                "NEXOFOLIO_CATALOG_MODEL_BASE_URL",
                format!("{}/v1", upstream.base),
            )
            .env("NEXOFOLIO_CATALOG_MODEL", "fixture")
            .env("NEXOFOLIO_CATALOG_MODEL_API_KEY", "fixture-model-key")
            .env("NEXOFOLIO_MODEL_MAX_CONCURRENCY", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let address = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let line = lines
                    .next_line()
                    .await
                    .unwrap()
                    .expect("API exited before startup");
                let value: Value = serde_json::from_str(&line).unwrap();
                if value["fields"]["message"] == "api_started" {
                    break value["fields"]["address"].as_str().unwrap().to_owned();
                }
            }
        })
        .await
        .unwrap();
        let drain =
            tokio::spawn(async move { while lines.next_line().await.ok().flatten().is_some() {} });
        let url = format!("http://{address}");
        let client = reqwest::Client::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if client
                    .get(format!("{url}/health/ready"))
                    .send()
                    .await
                    .is_ok_and(|r| r.status().is_success())
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        Self { url, child, drain }
    }
    pub async fn kill(&mut self) {
        self.child.start_kill().unwrap();
        self.child.wait().await.unwrap();
    }
    pub async fn stop(&mut self) {
        Command::new("kill")
            .args(["-TERM", &self.child.id().unwrap().to_string()])
            .status()
            .await
            .unwrap();
        let status = tokio::time::timeout(Duration::from_secs(5), self.child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success());
    }
}
impl Drop for ApiProcess {
    fn drop(&mut self) {
        self.drain.abort();
    }
}
