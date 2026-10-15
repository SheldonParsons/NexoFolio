//! Isolated real HTTP/API/PostgreSQL recovery, with a separately opted-in wall-clock soak.
#[allow(dead_code)]
#[path = "curation_restart/fixture.rs"]
mod fixture;
#[path = "curation_retry/upstream.rs"]
mod upstream;
use nexofolio_backend::wiring::{Config, Databases};
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::{
    endpoint::EndpointReader, knowledge::*, observation::*, testing::sample_exchange,
};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_adapter::PostgresKnowledge;
use nexofolio_observe::Observe;
use nexofolio_observe_adapter::PostgresObserve;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
type K = Knowledge<PostgresKnowledge, Observe<PostgresObserve>>;
struct Harness {
    binary: PathBuf,
    db: Databases,
    db_url: String,
    api: fixture::ApiProcess,
    up: upstream::Upstream,
    client: reqwest::Client,
    token: String,
    projects: Vec<ProjectId>,
    endpoints: Vec<EndpointId>,
    environments: Vec<EnvironmentId>,
    knowledge: K,
    out: PathBuf,
}
impl Harness {
    async fn start(name: &str) -> Self {
        let out = PathBuf::from(std::env::var("RETRY_EVIDENCE_DIR").expect("evidence directory"))
            .join(name);
        assert!(out.starts_with("/tmp/nexofolio-step02"));
        let up = upstream::Upstream::start(out.clone()).await;
        let db_url = fixture::database_url();
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
        let version = json!({
            "binary_sha256": Sha256::digest(std::fs::read(&binary).unwrap()).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
            "source_manifest": std::env::var("RETRY_SOURCE_MANIFEST").ok().and_then(|path|std::fs::read_to_string(path).ok()).and_then(|text|serde_json::from_str::<Value>(&text).ok()),
            "captured_at": chrono::Utc::now(),
        });
        std::fs::write(
            out.join("version.json"),
            serde_json::to_vec_pretty(&version).unwrap(),
        )
        .unwrap();
        let api = fixture::ApiProcess::start_binary(&db_url, &up.base, &binary).await;
        let client = reqwest::Client::new();
        let login: Value = client
            .post(format!("{}/v1/auth/login", api.url))
            .json(&json!({"account":"operator","password":"fixture-pass"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let token = login["token"].as_str().unwrap().to_string();
        let rows: Value = client
            .get(format!("{}/v1/projects", api.url))
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
            .map(|v| v["project_id"].as_str().unwrap().parse().unwrap())
            .collect();
        assert_eq!(projects.len(), 2);
        let observe = Observe::new(db.observe.clone());
        let mut endpoints = vec![];
        let mut environments = vec![];
        for (i, project) in projects.iter().enumerate() {
            let env: Value = client
                .post(format!("{}/v1/projects/{project}/environments", api.url))
                .bearer_auth(&token)
                .json(&json!({"name":"隔离恢复测试"}))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            let environment = env["id"]
                .as_str()
                .unwrap()
                .parse::<EnvironmentId>()
                .unwrap();
            environments.push(environment);
            let mut observation = sample_exchange(*project, environment);
            observation.observed_at = chrono::Utc::now();
            let Fact::Exchange(exchange) = &mut observation.fact else {
                unreachable!()
            };
            exchange.request.method = "POST".into();
            exchange.request.url = format!("https://synthetic.example/retry-{i}");
            exchange.request.headers = None;
            exchange.response = None;
            exchange.request.body = Body::Full {
                media_type: Some("application/json".into()),
                content: Content::Text(
                    if i == 0 {
                        json!({"f1":"a","f2":"b","f3":"c"})
                    } else {
                        json!({"other":"x"})
                    }
                    .to_string(),
                ),
            };
            observe.accept(vec![observation]).await.unwrap();
            endpoints.push(observe.list(*project).await.unwrap()[0].id);
        }
        let knowledge = Knowledge::new(db.knowledge.clone(), observe);
        let h = Self {
            binary,
            db,
            db_url,
            api,
            up,
            client,
            token,
            projects,
            endpoints,
            environments,
            knowledge,
            out,
        };
        h.manifest();
        h
    }
    fn write(&self, name: &str, value: Value) {
        std::fs::write(
            self.out.join(name),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
    fn manifest(&self) {
        self.write("fixture.json",json!({"api":self.api.url,"projects":self.projects,"endpoints":self.endpoints,"environments":self.environments}));
    }
    async fn submit(&self, index: usize) -> CurationTask {
        let p = self.projects[index];
        let e = self.endpoints[index];
        let response = self
            .client
            .post(format!(
                "{}/v1/projects/{p}/curate/describe/{e}",
                self.api.url
            ))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 202);
        let receipt: Value = response.json().await.unwrap();
        self.write(&format!("receipt-{index}.json"), receipt.clone());
        self.knowledge
            .task(p, receipt["task_id"].as_str().unwrap().parse().unwrap())
            .await
            .unwrap()
    }
    async fn wait(&self, t: &CurationTask, state: TaskState, seconds: u64) -> CurationTask {
        tokio::time::timeout(Duration::from_secs(seconds), async {
            loop {
                let v = self.knowledge.task(t.project_id, t.id).await.unwrap();
                if v.state == state {
                    break v;
                }
                assert!(
                    !matches!(v.state, TaskState::Failed | TaskState::Stopped),
                    "unexpected {v:?}"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("task did not reach {state:?}"))
    }
    async fn restart(&mut self) {
        self.api.kill().await;
        self.api =
            fixture::ApiProcess::start_binary(&self.db_url, &self.up.base, &self.binary).await;
        self.manifest();
    }
    async fn close(mut self) {
        self.api.stop().await;
        self.db.close().await;
    }
}
#[tokio::test]
#[ignore = "private PostgreSQL, real process fault smoke"]
async fn temporary_wait_releases_slot_and_restart_preserves_deadline() {
    let mut h = Harness::start("smoke").await;
    let t = h.submit(0).await;
    let waiting = h.wait(&t, TaskState::Waiting, 30).await;
    assert!(waiting.next_retry_at.is_some());
    assert_eq!(waiting.retry_reason.as_deref(), Some("model_unavailable"));
    let c = h
        .knowledge
        .candidate(t.project_id, waiting.candidate_id.unwrap())
        .await
        .unwrap();
    assert_eq!(c.view.fields.len(), 3);
    assert!(c.execution_input.is_some());
    h.write("before.json", json!({"task":waiting,"candidate":c}));
    let calls = h.up.model.calls.lock().unwrap().len();
    h.restart().await;
    let after = h.knowledge.task(t.project_id, t.id).await.unwrap();
    assert_eq!(after.next_retry_at, waiting.next_retry_at);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(h.up.model.calls.lock().unwrap().len(), calls);
    let other = h.submit(1).await;
    h.wait(&other, TaskState::Done, 15).await;
    h.up.model.mode("ok");
    let done = h.wait(&t, TaskState::Done, 90).await;
    assert_eq!(done.candidate_id, waiting.candidate_id);
    let final_c = h
        .knowledge
        .candidate(t.project_id, done.candidate_id.unwrap())
        .await
        .unwrap();
    assert_eq!(final_c.view.fields.len(), 4);
    assert_eq!(c.execution_input, final_c.execution_input);
    assert!(
        final_c
            .batches
            .iter()
            .flat_map(|b| &b.commands)
            .filter(|c| matches!(c,Command::PutFieldNote{path,..} if path=="request_body:f1"))
            .count()
            == 1
    );
    h.write("result.json",json!({"passed":true,"task":done,"candidate":final_c,"calls":h.up.model.calls.lock().unwrap().len()}));
    h.close().await;
}
#[tokio::test]
#[ignore = "REQUIRED real continuous two-hour failure; never virtual time"]
async fn two_hour_outage_automatically_resumes_original_candidate() {
    let mut h = Harness::start("two-hour").await;
    let t = h.submit(0).await;
    let initial = h.wait(&t, TaskState::Waiting, 30).await;
    let candidate = h
        .knowledge
        .candidate(t.project_id, initial.candidate_id.unwrap())
        .await
        .unwrap();
    assert_eq!(candidate.view.fields.len(), 3);
    let start = Instant::now();
    let wall_start = chrono::Utc::now();
    h.write(
        "start.json",
        json!({"at":wall_start,"task":initial,"candidate":candidate}),
    );
    let mut restarted = false;
    while start.elapsed() < Duration::from_secs(7200) {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let current = h.knowledge.task(t.project_id, t.id).await.unwrap();
        assert!(matches!(
            current.state,
            TaskState::Running | TaskState::Waiting
        ));
        assert_eq!(current.candidate_id, initial.candidate_id);
        let c = h
            .knowledge
            .candidate(t.project_id, initial.candidate_id.unwrap())
            .await
            .unwrap();
        assert_eq!(c.view.fields.len(), 3);
        assert_eq!(c.execution_input, candidate.execution_input);
        h.write("latest.json",json!({"wall":chrono::Utc::now(),"elapsed_seconds":start.elapsed().as_secs_f64(),"task":current,"calls":h.up.model.calls.lock().unwrap().len()}));
        if !restarted
            && start.elapsed() > Duration::from_secs(90)
            && current.state == TaskState::Waiting
        {
            h.restart().await;
            assert_eq!(
                h.knowledge
                    .task(t.project_id, t.id)
                    .await
                    .unwrap()
                    .next_retry_at,
                current.next_retry_at
            );
            restarted = true;
        }
    }
    let outage_seconds = start.elapsed().as_secs_f64();
    let recovery_at = chrono::Utc::now();
    assert!(outage_seconds >= 7200.0);
    h.up.model.mode("ok");
    let done = h.wait(&t, TaskState::Done, 420).await;
    let c = h
        .knowledge
        .candidate(t.project_id, initial.candidate_id.unwrap())
        .await
        .unwrap();
    assert_eq!(done.candidate_id, initial.candidate_id);
    assert_eq!(c.execution_input, candidate.execution_input);
    assert_eq!(c.view.fields.len(), 4);
    assert_eq!(
        c.batches
            .iter()
            .flat_map(|b| &b.commands)
            .filter(|c| matches!(c,Command::PutFieldNote{path,..} if path=="request_body:f1"))
            .count(),
        1
    );
    h.write("result.json",json!({"passed":true,"wall_start":wall_start,"recovery_at":recovery_at,"completed_at":chrono::Utc::now(),"continuous_failure_seconds":outage_seconds,"total_seconds":start.elapsed().as_secs_f64(),"restarted":restarted,"task":done,"candidate":c}));
    h.close().await;
}
#[tokio::test]
#[ignore = "private PostgreSQL, API fault/attention/stop matrix"]
async fn fault_attention_stop_and_partial_apply_matrix() {
    let mut h = Harness::start("matrix").await;
    let mut results = vec![];
    for (mode, reason) in [
        ("429", "model_rate_limited"),
        ("429date", "model_rate_limited"),
        ("408", "model_timeout"),
        ("500", "model_unavailable"),
        ("disconnect", "model_network"),
        ("timeout", "model_timeout"),
    ] {
        h.up.model.mode(mode);
        let started = Instant::now();
        let t = h.submit(0).await;
        let waiting = h.wait(&t, TaskState::Waiting, 190).await;
        assert_eq!(waiting.retry_reason.as_deref(), Some(reason));
        let delay = (waiting.next_retry_at.unwrap() - waiting.updated_at).num_seconds();
        if mode.starts_with("429") {
            assert!(delay >= 44);
        }
        if mode == "timeout" {
            assert!(started.elapsed() >= Duration::from_secs(149));
        }
        assert!(waiting.lease_until.is_none());
        h.knowledge.stop_task(t.project_id, t.id).await.unwrap();
        h.up.model.mode("ok");
        h.restart().await;
        let calls = h.up.model.calls.lock().unwrap().len();
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(h.up.model.calls.lock().unwrap().len(), calls);
        assert_eq!(
            h.knowledge.task(t.project_id, t.id).await.unwrap().state,
            TaskState::Stopped
        );
        results.push(json!({"mode":mode,"reason":reason,"seconds":started.elapsed().as_secs_f64(),"waiting":waiting,"stopped_after_restart":true}));
    }
    for mode in ["401", "invalid"] {
        h.up.model.mode(mode);
        let t = h.submit(0).await;
        let attention = h.wait(&t, TaskState::NeedsAttention, 30).await;
        assert!(attention.next_retry_at.is_none());
        let calls = h.up.model.calls.lock().unwrap().len();
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert_eq!(h.up.model.calls.lock().unwrap().len(), calls);
        h.up.model.mode("unknown");
        let resumed: Value = h
            .client
            .post(format!(
                "{}/v1/projects/{}/curate/candidates/{}/resume",
                h.api.url,
                t.project_id,
                attention.candidate_id.unwrap()
            ))
            .bearer_auth(&h.token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resumed["task_id"], t.id.to_string());
        let done = h.wait(&t, TaskState::Done, 30).await;
        let c = h
            .knowledge
            .candidate(t.project_id, done.candidate_id.unwrap())
            .await
            .unwrap();
        let field = c
            .view
            .fields
            .iter()
            .find(|f| f.path == "request_body:f2")
            .unwrap();
        assert_eq!(field.context.meaning.confirmation, Confirmation::Candidate);
        assert!(
            field
                .context
                .completion
                .as_ref()
                .is_some_and(|c| c.name.state == AspectState::Uncertain)
        );
        results
            .push(json!({"mode":mode,"attention":attention,"resumed":done,"unknown_field":field}));
    }
    h.up.model.mode("503");
    let t = h.submit(0).await;
    let waiting = h.wait(&t, TaskState::Waiting, 30).await;
    let before = h
        .knowledge
        .candidate(t.project_id, waiting.candidate_id.unwrap())
        .await
        .unwrap();
    assert_eq!(before.state, CandidateState::Partial);
    h.knowledge
        .apply_candidate_as(t.project_id, before.id, waiting.actor.clone())
        .await
        .unwrap();
    assert_eq!(
        h.knowledge.task(t.project_id, t.id).await.unwrap().state,
        TaskState::Stopped
    );
    h.up.model.mode("ok");
    h.restart().await;
    let calls = h.up.model.calls.lock().unwrap().len();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(h.up.model.calls.lock().unwrap().len(), calls);
    assert_eq!(
        h.knowledge
            .candidate(t.project_id, before.id)
            .await
            .unwrap()
            .state,
        CandidateState::Applied
    );
    results.push(json!({"partial_apply_stops_waiting":true,"task":t.id,"candidate":before.id}));
    h.write("result.json", json!({"passed":true,"cases":results}));
    h.close().await;
}
#[tokio::test]
#[ignore = "private PostgreSQL outage and actual lease recovery"]
async fn database_outage_keeps_acknowledged_results_and_fences_old_owner() {
    use nexofolio_knowledge_contracts::KnowledgeStore;
    let h = Harness::start("database-outage").await;
    h.up.model.mode("hold");
    let t = h.submit(0).await;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if h.up
                .model
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|v| v["paths"] == json!(["f2"]))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let running = h.knowledge.task(t.project_id, t.id).await.unwrap();
    let id = running.candidate_id.unwrap();
    let before = h.knowledge.candidate(t.project_id, id).await.unwrap();
    assert_eq!(before.view.fields.len(), 3);
    // Acquiring the same project lock proves no transaction is held across the blocked model IO.
    let mut tx = h.db.knowledge.begin().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), tx.lock_project(t.project_id))
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
    let mut admin_url = reqwest::Url::parse(&h.db_url).unwrap();
    let name = admin_url.path().trim_start_matches('/').to_string();
    assert!(
        name.starts_with("nexofolio_recovery_")
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    );
    admin_url.set_path("postgres");
    let admin = sqlx::PgPool::connect(admin_url.as_str()).await.unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER DATABASE {name} ALLOW_CONNECTIONS false"
    )))
    .execute(&admin)
    .await
    .unwrap();
    sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=$1 AND pid<>pg_backend_pid()").bind(&name).execute(&admin).await.unwrap();
    h.up.model.release.notify_waiters();
    tokio::time::sleep(Duration::from_secs(8)).await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER DATABASE {name} ALLOW_CONNECTIONS true"
    )))
    .execute(&admin)
    .await
    .unwrap();
    h.up.model.mode("ok");
    let done = h.wait(&t, TaskState::Done, 180).await;
    assert_eq!(done.candidate_id, Some(id));
    assert!(done.generation > running.generation);
    let final_c = h.knowledge.candidate(t.project_id, id).await.unwrap();
    assert_eq!(final_c.execution_input, before.execution_input);
    assert_eq!(final_c.view.fields.len(), 4);
    assert!(h.knowledge.heartbeat_task(&running).await.is_err());
    assert!(
        h.knowledge
            .checkpoint_candidate(
                t.project_id,
                id,
                CandidateState::Running,
                json!({"run_id":running.run_id})
            )
            .await
            .is_err()
    );
    assert_eq!(
        final_c
            .batches
            .iter()
            .flat_map(|b| &b.commands)
            .filter(|c| matches!(c,Command::PutFieldNote{path,..} if path=="request_body:f1"))
            .count(),
        1
    );
    h.write("result.json",json!({"passed":true,"before":running,"after":done,"no_lock_across_model":true,"candidate":final_c}));
    admin.close().await;
    h.close().await;
}

#[tokio::test]
#[ignore = "private API fixture for actual isolated Chrome acceptance"]
async fn serve_retry_browser_fixture() {
    let h = Harness::start("browser").await;
    let stop = h.out.join("stop");
    tokio::time::timeout(Duration::from_secs(3600), async {
        while !stop.exists() {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap();
    h.close().await;
}
#[tokio::test]
#[ignore = "real describe and organize temporary failures"]
async fn description_and_directory_use_same_durable_recovery() {
    let mut h = Harness::start("stages").await;
    let mut results = vec![];
    h.up.model.mode("describe503");
    let t = h.submit(0).await;
    let waiting = h.wait(&t, TaskState::Waiting, 30).await;
    h.up.model.mode("ok");
    let done = h.wait(&t, TaskState::Done, 70).await;
    assert_eq!(done.candidate_id, waiting.candidate_id);
    results.push(json!({"stage":"describe","waiting":waiting,"done":done}));
    h.restart().await;
    h.up.model.mode("organize503");
    let response = h
        .client
        .post(format!(
            "{}/v1/projects/{}/curate",
            h.api.url, h.projects[0]
        ))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    let receipt: Value = response.json().await.unwrap();
    let t = h
        .knowledge
        .task(
            h.projects[0],
            receipt["task_id"].as_str().unwrap().parse().unwrap(),
        )
        .await
        .unwrap();
    let waiting = h.wait(&t, TaskState::Waiting, 30).await;
    let c = h
        .knowledge
        .candidate(t.project_id, waiting.candidate_id.unwrap())
        .await
        .unwrap();
    assert_eq!(c.view.fields.len(), 4);
    let calls = h.up.model.calls.lock().unwrap().len();
    h.up.model.mode("ok");
    let done = h.wait(&t, TaskState::Done, 70).await;
    assert_eq!(done.candidate_id, waiting.candidate_id);
    assert!(
        h.up.model.calls.lock().unwrap()[calls..]
            .iter()
            .all(|call| call["stage"] == "organize")
    );
    results.push(json!({"stage":"organize","waiting":waiting,"done":done}));
    h.write("result.json", json!({"passed":true,"cases":results}));
    h.close().await;
}

#[tokio::test]
#[ignore = "real warm shared-cooldown fairness across projects"]
async fn shared_cooldown_does_not_starve_a_healthy_project() {
    let mut h = Harness::start("warm-fairness").await;
    let failing = h.submit(0).await;
    let waiting = h.wait(&failing, TaskState::Waiting, 30).await;
    let healthy = h.submit(1).await;
    let done = h.wait(&healthy, TaskState::Done, 150).await;
    let still_waiting = h.wait(&failing, TaskState::Waiting, 10).await;
    assert_eq!(still_waiting.candidate_id, waiting.candidate_id);
    assert!(
        h.up.model
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call["paths"] == json!(["other"]))
    );
    h.knowledge
        .stop_task(failing.project_id, failing.id)
        .await
        .unwrap();
    h.up.model.mode("ok");
    h.restart().await;
    let calls = h.up.model.calls.lock().unwrap().len();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(calls, h.up.model.calls.lock().unwrap().len());
    h.write("result.json", json!({"passed":true,"failing_project":still_waiting,"healthy_project":done,"stopped_after_restart":true}));
    h.close().await;
}

#[path = "curation_retry/diagnostics.rs"]
mod diagnostics;
