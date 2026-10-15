use super::*;
use nexofolio_backend::wiring::{Config, Databases};
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::{endpoint::EndpointReader, observation::*, testing::sample_exchange};
use nexofolio_observe::Observe;
use std::process::Stdio;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
};

pub struct Harness {
    pub db: Databases,
    pub sql: sqlx::PgPool,
    pub db_url: String,
    pub api: String,
    pub process: Child,
    drain: tokio::task::JoinHandle<()>,
    pub up: upstream::Upstream,
    pub client: reqwest::Client,
    pub token: String,
    pub projects: Vec<ProjectId>,
    pub endpoints: Vec<Vec<EndpointId>>,
    pub environments: Vec<EnvironmentId>,
    pub out: std::path::PathBuf,
}
impl Harness {
    pub async fn start() -> Self {
        let out = std::path::PathBuf::from(
            std::env::var("OPERATIONS_EVIDENCE_DIR").expect("isolated evidence path"),
        );
        assert!(out.starts_with("/tmp/nexofolio-operations"));
        let db_url = std::env::var("TEST_DATABASE_URL").expect("isolated database");
        let url = reqwest::Url::parse(&db_url).unwrap();
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert!(url.path().starts_with("/nexofolio_operations_"));
        let up = upstream::Upstream::start(out.clone()).await;
        let config = Config::from_lookup(|key| match key {
            "DATABASE_URL" => Some(db_url.clone()),
            "NEXOFOLIO_ZENTAO_BASE_URL" => Some(format!("{}/api.php/v1", up.base)),
            "NEXOFOLIO_SESSION_KEY" => Some("ab".repeat(32)),
            _ => None,
        })
        .unwrap();
        let db = Databases::new(&config).unwrap();
        db.migrate().await.unwrap();
        let binary = out.join("api-runtime");
        std::fs::copy(env!("CARGO_BIN_EXE_nexofolio-api"), &binary).unwrap();
        use sha2::{Digest, Sha256};
        std::fs::write(out.join("version.json"),serde_json::to_vec_pretty(&json!({"binary_sha256":Sha256::digest(std::fs::read(&binary).unwrap()).iter().map(|b|format!("{b:02x}")).collect::<String>(),"captured_at":chrono::Utc::now(),"source_manifest":std::env::var("OPERATIONS_SOURCE_MANIFEST").ok().and_then(|p|std::fs::read_to_string(p).ok()).and_then(|s|serde_json::from_str::<Value>(&s).ok())})).unwrap()).unwrap();
        let mut process = Command::new(&binary)
            .env_clear()
            .env("DATABASE_URL", &db_url)
            .env("NEXOFOLIO_BIND_ADDR", "127.0.0.1:0")
            .env("NEXOFOLIO_LOG", "info")
            .env(
                "NEXOFOLIO_ZENTAO_BASE_URL",
                format!("{}/api.php/v1", up.base),
            )
            .env("NEXOFOLIO_SESSION_KEY", "ab".repeat(32))
            .env(
                "NEXOFOLIO_CATALOG_MODEL_BASE_URL",
                format!("{}/v1", up.base),
            )
            .env("NEXOFOLIO_CATALOG_MODEL", "fixture")
            .env("NEXOFOLIO_CATALOG_MODEL_API_KEY", "fixture-model-key")
            .env("NEXOFOLIO_MODEL_MAX_CONCURRENCY", "2")
            .env("NEXOFOLIO_COLLECT_BURST", "2")
            .env("NEXOFOLIO_COLLECT_RATE_PER_MINUTE", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(process.stdout.take().unwrap()).lines();
        let address = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let value: Value =
                    serde_json::from_str(&lines.next_line().await.unwrap().expect("API alive"))
                        .unwrap();
                if value["fields"]["message"] == "api_started" {
                    break value["fields"]["address"].as_str().unwrap().to_owned();
                }
            }
        })
        .await
        .unwrap();
        let drain =
            tokio::spawn(async move { while lines.next_line().await.ok().flatten().is_some() {} });
        let api = format!("http://{address}");
        let client = reqwest::Client::new();
        let token = login(&client, &api, "A80646").await;
        let rows: Value = client
            .get(format!("{api}/v1/projects"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let projects: Vec<ProjectId> = rows["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["project_id"].as_str().unwrap().parse().unwrap())
            .collect();
        let observe = Observe::new(db.observe.clone());
        let mut endpoints = vec![];
        let mut environments = vec![];
        for project in &projects {
            let env: Value = client
                .post(format!("{api}/v1/projects/{project}/environments"))
                .bearer_auth(&token)
                .json(&json!({"name":"运行状态隔离验收"}))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            let environment: EnvironmentId = env["id"].as_str().unwrap().parse().unwrap();
            environments.push(environment);
            for n in 0..4 {
                let mut observation = sample_exchange(*project, environment);
                observation.observed_at = chrono::Utc::now();
                let Fact::Exchange(exchange) = &mut observation.fact else {
                    unreachable!()
                };
                exchange.request.method = "POST".into();
                exchange.request.url = format!("https://synthetic.example/operations-{n}");
                exchange.request.headers = None;
                exchange.response = None;
                exchange.request.body = Body::Full {
                    media_type: Some("application/json".into()),
                    content: Content::Text(json!({"f1":"a","f2":"b","f3":"c"}).to_string()),
                };
                observe.accept(vec![observation]).await.unwrap();
            }
            endpoints.push(
                observe
                    .list(*project)
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|e| e.id)
                    .collect(),
            );
        }
        let sql = sqlx::PgPool::connect(&db_url).await.unwrap();
        let h = Self {
            db,
            sql,
            db_url,
            api,
            process,
            drain,
            up,
            client,
            token,
            projects,
            endpoints,
            environments,
            out,
        };
        h.write("fixture.json", json!({"api":h.api,"pid":h.process.id(),"projects":h.projects,"endpoints":h.endpoints,"environments":h.environments}));
        h
    }
    pub fn write(&self, name: &str, value: Value) {
        std::fs::write(
            self.out.join(name),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
    pub async fn get(&self, path: &str) -> Value {
        self.client
            .get(format!("{}{path}", self.api))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }
    pub async fn submit(&self, index: usize, all: bool) -> uuid::Uuid {
        let project = self.projects[index];
        let suffix = if all {
            String::new()
        } else {
            format!("/describe/{}", self.endpoints[index][0])
        };
        let r = self
            .client
            .post(format!("{}/v1/projects/{project}/curate{suffix}", self.api))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 202);
        let r: Value = r.json().await.unwrap();
        r["task_id"].as_str().unwrap().parse().unwrap()
    }
    pub async fn state(&self, id: uuid::Uuid) -> String {
        sqlx::query_scalar("SELECT state FROM knowledge.curation_tasks WHERE id=$1")
            .bind(id)
            .fetch_one(&self.sql)
            .await
            .unwrap()
    }
    pub async fn wait(&self, id: uuid::Uuid, state: &str) {
        tokio::time::timeout(Duration::from_secs(90), async {
            while self.state(id).await != state {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("task {id} did not reach {state}"));
    }
    pub async fn stop(&self, project: ProjectId, id: uuid::Uuid) {
        assert_eq!(
            self.client
                .post(format!(
                    "{}/v1/projects/{project}/curate/tasks/{id}/stop",
                    self.api
                ))
                .bearer_auth(&self.token)
                .send()
                .await
                .unwrap()
                .status(),
            204
        );
    }
    pub async fn close(mut self) {
        self.up.model.release.notify_waiters();
        Command::new("kill")
            .args(["-TERM", &self.process.id().unwrap().to_string()])
            .status()
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(10), self.process.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        self.drain.abort();
        self.db.close().await;
        self.sql.close().await;
        std::fs::remove_file(self.out.join("api-runtime")).unwrap();
    }
}
pub async fn login(client: &reqwest::Client, api: &str, account: &str) -> String {
    let value: Value = client
        .post(format!("{api}/v1/auth/login"))
        .json(&json!({"account":account,"password":"fixture-pass"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    value["token"].as_str().unwrap().to_owned()
}
