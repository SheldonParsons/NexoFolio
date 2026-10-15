use nexofolio_common::ProjectId;
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_knowledge::Knowledge;
use serde_json::{Value, json};
use std::time::Duration;
#[allow(dead_code)]
#[path = "curation_restart/fixture.rs"]
mod fixture;
#[tokio::test]
#[ignore = "isolated TEST_DATABASE_URL required"]
async fn accepted_task_keeps_execution_audit_events() {
    use nexofolio_common::EnvironmentId;
    use nexofolio_contracts::{observation::*, testing::sample_exchange};
    use nexofolio_observe::Observe;
    use std::sync::atomic::Ordering;
    let url = fixture::database_url();
    let upstream = fixture::Upstream::start().await;
    upstream.model.block.store(false, Ordering::SeqCst);
    let db = upstream.databases(&url).await;
    let mut api = fixture::ApiProcess::start(&url, &upstream).await;
    let client = reqwest::Client::new();
    let login: Value = client
        .post(format!("{}/v1/auth/login", api.url))
        .json(&json!({"account":"operator","password":"fixture-pass"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = login["token"].as_str().unwrap();
    let projects: Value = client
        .get(format!("{}/v1/projects", api.url))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let project: ProjectId = projects["items"][0]["project_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let observe = Observe::new(db.observe.clone());
    let mut observation = sample_exchange(project, EnvironmentId::new());
    observation.observed_at = chrono::Utc::now();
    let Fact::Exchange(exchange) = &mut observation.fact else {
        unreachable!()
    };
    exchange.request.method = "POST".into();
    exchange.request.url = "https://synthetic.example/audit-review".into();
    exchange.request.headers = None;
    exchange.request.body = Body::Full {
        media_type: Some("application/json".into()),
        content: Content::Text(json!({"f1":"a"}).to_string()),
    };
    exchange.response = None;
    observe.accept(vec![observation]).await.unwrap();
    let endpoint = EndpointReader::list(&observe, project).await.unwrap()[0].id;
    let started = client
        .post(format!(
            "{}/v1/projects/{project}/curate/describe/{endpoint}",
            api.url
        ))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(started.status(), 202);
    let receipt: Value = started.json().await.unwrap();
    let task_id: uuid::Uuid = receipt["task_id"].as_str().unwrap().parse().unwrap();
    let knowledge = Knowledge::new(db.knowledge.clone(), observe);
    let task = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let t = knowledge.task(project, task_id).await.unwrap();
            if !t.state.unfinished() {
                break t;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(task.state, TaskState::Done);
    api.stop().await;
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT data->>'action' FROM audit.events WHERE project=$1 ORDER BY at,id",
    )
    .bind(project.to_string())
    .fetch_all(&pool)
    .await
    .unwrap();
    println!(
        "actual execution completed; persisted audit actions: {:?}",
        rows
    );
    let events: Vec<Value> = sqlx::query_scalar("SELECT data FROM audit.events WHERE project=$1")
        .bind(project.to_string())
        .fetch_all(&pool)
        .await
        .unwrap();
    for action in ["curation.start", "curation.running", "curation.finished"] {
        let event = events.iter().find(|e| e["action"] == action).unwrap();
        assert_eq!(event["execution"]["task_id"], json!(task_id));
        assert_eq!(
            event["actor"],
            format!("user:{}", task.actor.user_id().unwrap())
        );
        if action != "curation.start" {
            assert_eq!(event["execution"]["generation"], 1);
            assert_eq!(event["resource"], json!(task.candidate_id.unwrap()));
        }
        let text = event.to_string();
        assert!(
            !text.contains("fixture-pass") && !text.contains(token) && !text.contains("合成业务")
        );
    }
    pool.close().await;
    db.close().await;
    assert!(
        rows.iter().any(|a| a == "curation.start"),
        "HTTP admission audit should be present"
    );
    assert!(
        rows.iter().any(|a| a == "curation.running"),
        "queued execution lost running audit event"
    );
    assert!(
        rows.iter().any(|a| a == "curation.finished"),
        "queued execution lost terminal audit event"
    );
}

#[tokio::test]
#[ignore = "isolated TEST_DATABASE_URL required"]
async fn queue_stop_and_execution_failure_keep_safe_audit_identity() {
    use nexofolio_common::EnvironmentId;
    use nexofolio_contracts::{observation::*, testing::sample_exchange};
    use nexofolio_observe::Observe;
    use std::sync::atomic::Ordering;
    let url = fixture::database_url();
    let upstream = fixture::Upstream::start().await;
    let db = upstream.databases(&url).await;
    let mut api = fixture::ApiProcess::start(&url, &upstream).await;
    let client = reqwest::Client::new();
    let login: Value = client
        .post(format!("{}/v1/auth/login", api.url))
        .json(&json!({"account":"operator","password":"fixture-pass"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = login["token"].as_str().unwrap();
    let projects: Value = client
        .get(format!("{}/v1/projects", api.url))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let p: ProjectId = projects["items"][0]["project_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let observe = Observe::new(db.observe.clone());
    let mut observation = sample_exchange(p, EnvironmentId::new());
    observation.observed_at = chrono::Utc::now();
    let Fact::Exchange(exchange) = &mut observation.fact else {
        unreachable!()
    };
    exchange.request.method = "POST".into();
    exchange.request.url = "https://synthetic.example/blocked-audit".into();
    exchange.request.headers = None;
    exchange.response = None;
    exchange.request.body = Body::Full {
        media_type: Some("application/json".into()),
        content: Content::Text(json!({"f1":"a","f2":"b","f3":"c"}).to_string()),
    };
    observe.accept(vec![observation]).await.unwrap();
    let endpoint = EndpointReader::list(&observe, p).await.unwrap()[0].id;
    let k = Knowledge::new(db.knowledge.clone(), observe);
    let base = format!("{}/v1/projects/{p}/curate", api.url);
    let receipt: Value = client
        .post(format!("{base}/describe/{endpoint}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let a: uuid::Uuid = receipt["task_id"].as_str().unwrap().parse().unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        while !upstream.model.waiting.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(20)).await
        }
    })
    .await
    .unwrap();
    let receipt: Value = client
        .post(&base)
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let b: uuid::Uuid = receipt["task_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        client
            .post(format!("{base}/tasks/{b}/stop"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    let receipt: Value = client
        .post(&base)
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let failed: uuid::Uuid = receipt["task_id"].as_str().unwrap().parse().unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query("DELETE FROM access.user_project_access WHERE project_id=$1")
        .bind(uuid::Uuid::parse_str(&p.to_string()).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    k.stop_task(p, a).await.unwrap();
    upstream.model.block.store(false, Ordering::SeqCst);
    upstream.model.release.notify_waiters();
    tokio::time::timeout(Duration::from_secs(10),async{loop{let total:i64=sqlx::query_scalar("SELECT count(*) FROM audit.events WHERE project=$1 AND data->>'action'='curation.finished'").bind(p.to_string()).fetch_one(&pool).await.unwrap();if total>=2{break;}tokio::time::sleep(Duration::from_millis(30)).await}}).await.unwrap();
    assert_eq!(
        k.task(p, failed).await.unwrap().state,
        TaskState::NeedsAttention
    );
    let events: Vec<Value> = sqlx::query_scalar("SELECT data FROM audit.events WHERE project=$1")
        .bind(p.to_string())
        .fetch_all(&pool)
        .await
        .unwrap();
    let find = |task: uuid::Uuid, action: &str| {
        events
            .iter()
            .find(|e| e["execution"]["task_id"] == json!(task) && e["action"] == action)
            .unwrap()
    };
    assert_eq!(find(b, "curation.cancel")["status"], 204);
    assert_eq!(find(a, "curation.finished")["code"], "STOPPED");
    assert_eq!(find(failed, "curation.finished")["kind"], "error");
    assert!(
        !events.iter().any(
            |e| e["execution"]["task_id"] == json!(failed) && e["action"] == "curation.running"
        )
    );
    for event in events {
        assert_eq!(
            event["actor"],
            format!(
                "user:{}",
                k.task(p, a).await.unwrap().actor.user_id().unwrap()
            )
        );
        assert!(!event.to_string().contains(token));
    }
    // Read-time overlay must also repair pre-fix checkpoints without rewriting history.
    sqlx::query("UPDATE knowledge.candidates SET progress=jsonb_set(progress,'{task_state}','\"running\"'::jsonb) WHERE project_id=$1").bind(uuid::Uuid::parse_str(&p.to_string()).unwrap()).execute(&pool).await.unwrap();
    let login: Value = client
        .post(format!("{}/v1/auth/login", api.url))
        .json(&json!({"account":"operator","password":"fixture-pass"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let current: Value = client
        .get(format!("{base}?task_id={a}"))
        .bearer_auth(login["token"].as_str().unwrap())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(current["task_state"], "stopped");
    api.stop().await;
    pool.close().await;
    db.close().await;
}
