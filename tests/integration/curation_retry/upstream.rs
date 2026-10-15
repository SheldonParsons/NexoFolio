use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::{net::TcpListener, task::JoinHandle};
pub struct Model {
    pub release: tokio::sync::Notify,
    pub management: std::sync::atomic::AtomicBool,
    pub mode: Mutex<String>,
    pub calls: Mutex<Vec<Value>>,
    pub start: Instant,
    pub out: PathBuf,
}
impl Model {
    pub fn mode(&self, mode: &str) {
        *self.mode.lock().unwrap() = mode.into();
    }
    pub fn record(&self, event: Value) {
        self.calls.lock().unwrap().push(event.clone());
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.out.join("calls.jsonl"))
            .unwrap();
        writeln!(file, "{event}").unwrap();
    }
}
async fn complete(State(s): State<Arc<Model>>, Json(body): Json<Value>) -> Response {
    let prompt = body["messages"][0]["content"].as_str().unwrap();
    let raw = prompt.split("字段和可核查的证据：").nth(1);
    let inputs: Vec<Value> = raw
        .map(|r| serde_json::from_str(r.lines().next().unwrap()).unwrap())
        .unwrap_or_default();
    let paths: Vec<_> = inputs.iter().map(|i| i["path"].as_str().unwrap()).collect();
    let mode = std::fs::read_to_string(s.out.join("control.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| v["mode"].as_str().map(str::to_string))
        .unwrap_or_else(|| s.mode.lock().unwrap().clone());
    let directory = prompt.contains("接口列表:");
    let fault = paths == ["f2"]
        || (mode.starts_with("describe") && raw.is_none() && !directory)
        || (mode.starts_with("organize") && directory);
    let status = if fault && matches!(mode.as_str(), "503" | "429" | "401" | "408" | "500") {
        mode.parse().unwrap()
    } else if fault && (mode == "429date" || mode == "describe429") {
        429
    } else if fault && (mode == "describe503" || mode == "organize503") {
        503
    } else {
        200
    };
    s.record(json!({"at":chrono::Utc::now(),"monotonic_ms":s.start.elapsed().as_millis(),"paths":paths,"mode":mode,"status":status,"stage":if raw.is_some(){"fields"}else if directory{"organize"}else{"describe"}}));
    if status != 200 {
        let retry = if mode == "429date" {
            (chrono::Utc::now() + chrono::TimeDelta::seconds(45))
                .format("%a, %d %b %Y %H:%M:%S GMT")
                .to_string()
        } else if mode == "429" {
            "45".into()
        } else {
            "30".into()
        };
        return (
            StatusCode::from_u16(status).unwrap(),
            [("retry-after", retry)],
            "synthetic failure",
        )
            .into_response();
    }
    if fault && mode == "disconnect" {
        return Response::builder()
            .status(200)
            .header("content-length", "100")
            .body(axum::body::Body::new(BrokenBody))
            .unwrap();
    }
    if fault && mode == "hold" {
        s.release.notified().await;
    }
    if fault && mode == "timeout" {
        tokio::time::sleep(std::time::Duration::from_secs(180)).await;
    }
    let content = if fault && mode == "invalid" {
        "invalid".into()
    } else if raw.is_some() {
        json!({"fields":inputs.iter().filter(|i|i["path"]!="f2" || mode=="ok" || mode=="unknown" || (fault && (mode=="hold" || mode=="timeout")) || mode.starts_with("organize")).map(|i|if mode=="unknown" {json!({"key":i["key"],"name_unknown":"资料没有名称依据","description_unknown":"资料不足，无法核实业务含义"})}else{json!({"key":i["key"],"display_name":"合成字段","description":"可核查的合成资料说明"})}).collect::<Vec<_>>()}).to_string()
    } else if directory {
        let rows: Vec<Value> = serde_json::from_str(
            prompt
                .split("接口列表:\n")
                .nth(1)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        json!({"folders":[],"unplaced":rows.iter().map(|row|json!({"key":row["key"],"reason":"合成资料不足以分类"})).collect::<Vec<_>>()}).to_string()
    } else {
        json!({"name":"合成恢复接口","purpose":"用于隔离故障验收"}).to_string()
    };
    Json(json!({"choices":[{"message":{"content":content},"finish_reason":"stop"}]}))
        .into_response()
}
async fn token() -> Json<Value> {
    Json(json!({"token":"fixture-upstream-token"}))
}
async fn user(State(s): State<Arc<Model>>) -> Json<Value> {
    let account = if s.management.load(std::sync::atomic::Ordering::SeqCst) {
        "a80646"
    } else {
        "operator"
    };
    Json(
        json!({"profile":{"id":1,"account":account,"realname":"隔离发起人","deleted":"0","view":{"projects":"11,12"}}}),
    )
}
async fn projects() -> Json<Value> {
    Json(
        json!({"page":1,"limit":100,"total":2,"projects":[{"id":11,"name":"持续故障验收","status":"doing"},{"id":12,"name":"其他项目验收","status":"doing"}]}),
    )
}
pub struct Upstream {
    pub base: String,
    pub model: Arc<Model>,
    task: JoinHandle<()>,
}
impl Upstream {
    pub async fn start(out: PathBuf) -> Self {
        std::fs::create_dir_all(&out).unwrap();
        let model = Arc::new(Model {
            release: tokio::sync::Notify::new(),
            management: std::sync::atomic::AtomicBool::new(false),
            mode: Mutex::new("503".into()),
            calls: Mutex::new(vec![]),
            start: Instant::now(),
            out,
        });
        let app = Router::new()
            .route("/api.php/v1/tokens", post(token))
            .route("/api.php/v1/user", get(user))
            .route("/api.php/v1/projects", get(projects))
            .route("/v1/chat/completions", post(complete))
            .with_state(model.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { base, model, task }
    }
}
impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct BrokenBody;
impl http_body::Body for BrokenBody {
    type Data = axum::body::Bytes;
    type Error = std::io::Error;
    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        std::task::Poll::Ready(Some(Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "synthetic disconnect",
        ))))
    }
}
