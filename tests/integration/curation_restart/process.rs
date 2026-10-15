use super::fixture::{ApiProcess, Upstream};
use nexofolio_common::{EnvironmentId, ProjectId};
use nexofolio_contracts::{
    endpoint::EndpointReader, knowledge::*, observation::*, testing::sample_exchange,
};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::KnowledgeStore;
use nexofolio_observe::Observe;
use serde_json::{Value, json};
use std::{sync::atomic::Ordering, time::Duration};

pub async fn run() {
    let db = super::fixture::database_url();
    let upstream = Upstream::start().await;
    let databases = upstream.databases(&db).await;
    let mut old = ApiProcess::start(&db, &upstream).await;
    let client = reqwest::Client::new();
    let login = client
        .post(format!("{}/v1/auth/login", old.url))
        .json(&json!({"account":"operator","password":"fixture-pass"}))
        .send()
        .await
        .unwrap();
    assert!(login.status().is_success());
    let login: Value = login.json().await.unwrap();
    let token = login["token"].as_str().unwrap();
    let projects: Value = client
        .get(format!("{}/v1/projects", old.url))
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
    let observe = Observe::new(databases.observe.clone());
    let mut observation = sample_exchange(project, EnvironmentId::new());
    observation.observed_at = chrono::Utc::now();
    let Fact::Exchange(exchange) = &mut observation.fact else {
        unreachable!()
    };
    exchange.request.method = "POST".into();
    exchange.request.url = "https://synthetic.example/restart".into();
    exchange.request.headers = None;
    exchange.request.body = Body::Full {
        media_type: Some("application/json".into()),
        content: Content::Text(json!({"f1":"a","f2":"b","f3":"c"}).to_string()),
    };
    exchange.response = None;
    observe.accept(vec![observation]).await.unwrap();
    let endpoint = observe.list(project).await.unwrap()[0].id;
    let response = client
        .post(format!(
            "{}/v1/projects/{project}/curate/describe/{endpoint}",
            old.url
        ))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    let progress: Value = response.json().await.unwrap();
    let task_id: uuid::Uuid = progress["task_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(progress["task_state"], "queued");
    assert!(progress["candidate_id"].is_null());
    let knowledge = Knowledge::new(databases.knowledge.clone(), observe);
    let id = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(id) = knowledge.task(project, task_id).await.unwrap().candidate_id {
                break id;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let before = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let c = knowledge.candidate(project, id).await.unwrap();
            let has = |key: &str| c.view.fields.iter().any(|f| f.path == key);
            if c.execution_input.is_some()
                && has("request_body:f1")
                && has("request_body:f3")
                && !has("request_body:f2")
                && upstream.model.waiting.load(Ordering::SeqCst)
            {
                break c;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(before.state, CandidateState::Running);
    // Worker occupied: a different scope is durably accepted without input/candidate creation.
    let queued: Value = client
        .post(format!("{}/v1/projects/{project}/curate", old.url))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut queued_id: uuid::Uuid = queued["task_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(queued["task_state"], "queued");
    assert!(queued["candidate_id"].is_null());
    let duplicate: Value = client
        .post(format!("{}/v1/projects/{project}/curate/organize", old.url))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(duplicate["task_id"], queued["task_id"]);
    assert_eq!(
        client
            .get(format!("{}/v1/projects/{project}/curate/tasks", old.url))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("{}/v1/projects/{project}/curate/tasks", old.url))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let task_url = format!("{}/v1/projects/{project}/curate/tasks/{queued_id}", old.url);
    assert_eq!(client.get(&task_url).send().await.unwrap().status(), 401);
    assert_eq!(
        client
            .post(format!("{task_url}/stop"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(&task_url)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .post(format!("{task_url}/stop"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        knowledge.task(project, queued_id).await.unwrap().state,
        TaskState::Stopped
    );
    let stopped_task_id = queued_id;
    let requeued: Value = client
        .post(format!("{}/v1/projects/{project}/curate", old.url))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    queued_id = requeued["task_id"].as_str().unwrap().parse().unwrap();
    assert_ne!(queued_id, stopped_task_id);
    let pool = sqlx::PgPool::connect(&db).await.unwrap();
    // Session expiry is independent from accepted task authorization. Scope uses current account grants.
    sqlx::query("UPDATE access.internal_sessions SET issued_at=clock_timestamp()-interval '2 seconds',expires_at=clock_timestamp()-interval '1 second'").execute(&pool).await.unwrap();
    assert_eq!(
        client
            .get(&task_url)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    old.kill().await;
    assert_eq!(
        knowledge.candidate(project, id).await.unwrap().state,
        CandidateState::Running
    );
    let calls_before = upstream.model.paths.lock().unwrap().len();
    // Advance only this killed fixture's durable heartbeat; the 120-second
    // eligibility boundary is separately checked with the core test clock.
    let mut expired = knowledge.candidate(project, id).await.unwrap();
    expired.updated_at = chrono::Utc::now() - chrono::TimeDelta::seconds(121);
    let mut stopped = expired.clone();
    stopped.id = uuid::Uuid::new_v4();
    stopped.state = CandidateState::Interrupted;
    stopped.progress["message"] = json!("用户手动停止");
    let mut tx = databases.knowledge.begin().await.unwrap();
    tx.put_candidate(&expired).await.unwrap();
    let mut expired_task = tx.task(project, task_id).await.unwrap().unwrap();
    expired_task.lease_until = Some(expired.updated_at);
    tx.put_task(&expired_task).await.unwrap();
    tx.put_candidate(&stopped).await.unwrap();
    tx.commit().await.unwrap();
    upstream.model.block.store(false, Ordering::SeqCst);
    // Two fresh API instances use the real startup path, without a resume HTTP call.
    let (mut next, mut competing) = tokio::join!(
        ApiProcess::start(&db, &upstream),
        ApiProcess::start(&db, &upstream)
    );
    assert_eq!(
        knowledge.task(project, queued_id).await.unwrap().id,
        queued_id
    );
    // Keep the queued full-project scope out of this single-interface model fixture.
    knowledge.stop_task(project, queued_id).await.unwrap();
    let after = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let c = knowledge.candidate(project, id).await.unwrap();
            if c.state != CandidateState::Running {
                break c;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(after.state, CandidateState::Ready);
    assert_eq!(after.id, before.id);
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.execution_input, before.execution_input);
    assert_eq!(after.view.fields.len(), before.view.fields.len() + 1);
    for field in &before.view.fields {
        let saved = after
            .view
            .fields
            .iter()
            .find(|f| f.path == field.path)
            .unwrap();
        assert_eq!(
            serde_json::to_value(saved).unwrap(),
            serde_json::to_value(field).unwrap()
        );
    }
    assert_eq!(after.progress["fields_updated"], after.view.fields.len());
    assert_eq!(after.progress["attempts"], 2);
    assert_eq!(after.operations[0].actor, before.operations[0].actor);
    assert_eq!(
        after
            .operations
            .iter()
            .filter(|o| o.action == CandidateAction::Resume)
            .count(),
        1
    );
    assert!(matches!(
        after.operations.last().unwrap().actor,
        CandidateActor::System { .. }
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        &upstream.model.paths.lock().unwrap()[calls_before..],
        &[vec!["f2".to_owned()]]
    );
    let unchanged = knowledge.candidate(project, stopped.id).await.unwrap();
    assert_eq!(unchanged.state, CandidateState::Interrupted);
    assert_eq!(unchanged.operations.len(), stopped.operations.len());
    assert!(
        knowledge
            .endpoint(endpoint)
            .await
            .unwrap()
            .fields
            .is_empty()
    );
    assert_eq!(
        knowledge.task(project, task_id).await.unwrap().state,
        TaskState::Done
    );
    assert_eq!(
        knowledge.task(project, task_id).await.unwrap().generation,
        2
    );
    // Refresh synthetic login, then prove query/stop use current grants.
    let login: Value = client
        .post(format!("{}/v1/auth/login", next.url))
        .json(&json!({"account":"operator","password":"fixture-pass"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = login["token"].as_str().unwrap();
    let task_path = format!("{}/v1/projects/{project}/curate/tasks/{task_id}", next.url);
    sqlx::query("DELETE FROM access.user_project_access WHERE project_id=$1")
        .bind(uuid::Uuid::parse_str(&project.to_string()).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        client
            .get(&task_path)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .post(format!("{task_path}/stop"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .get(format!("{}/v1/projects/{project}/curate/tasks", next.url))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    // A previously accepted task cannot bypass revocation when its turn arrives.
    let denied = knowledge
        .submit_task(
            project,
            None,
            None,
            None,
            before.operations[0].actor.clone(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if knowledge.task(project, denied.id).await.unwrap().state == TaskState::NeedsAttention
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        knowledge
            .task(project, denied.id)
            .await
            .unwrap()
            .candidate_id
            .is_none()
    );
    assert_eq!(
        knowledge
            .task(project, stopped_task_id)
            .await
            .unwrap()
            .state,
        TaskState::Stopped
    );
    // Restore the fixture grant, then force only the queue storage operation to fail.
    let login: Value = client
        .post(format!("{}/v1/auth/login", next.url))
        .json(&json!({"account":"operator","password":"fixture-pass"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = login["token"].as_str().unwrap();
    sqlx::query("ALTER TABLE knowledge.curation_tasks RENAME TO queue_unavailable")
        .execute(&pool)
        .await
        .unwrap();
    let rejected = client
        .post(format!("{}/v1/projects/{project}/curate", next.url))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 503);
    assert!(
        rejected.json::<Value>().await.unwrap()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("尚未提交")
    );
    sqlx::query("ALTER TABLE knowledge.queue_unavailable RENAME TO curation_tasks")
        .execute(&pool)
        .await
        .unwrap();
    // Disable an account after durable submission, before dispatch begins.
    next.stop().await;
    competing.stop().await;
    let disabled = knowledge
        .submit_task(
            project,
            None,
            None,
            None,
            before.operations[0].actor.clone(),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE access.users SET enabled=false")
        .execute(&pool)
        .await
        .unwrap();
    let mut last = ApiProcess::start(&db, &upstream).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if knowledge.task(project, disabled.id).await.unwrap().state
                == TaskState::NeedsAttention
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        knowledge
            .task(project, disabled.id)
            .await
            .unwrap()
            .candidate_id
            .is_none()
    );
    last.stop().await;
    let events:Vec<Value>=sqlx::query_scalar("SELECT data FROM audit.events WHERE project=$1 AND data->'execution'->>'task_id'=$2 AND data->'execution'->>'generation'='2'").bind(project.to_string()).bind(task_id.to_string()).fetch_all(&pool).await.unwrap();
    for action in [
        "curation.recovered",
        "curation.running",
        "curation.finished",
    ] {
        let event = events.iter().find(|e| e["action"] == action).unwrap();
        assert_eq!(event["resource"], json!(id));
        assert_eq!(
            event["actor"],
            format!("user:{}", before.operations[0].actor.user_id().unwrap())
        );
    }
    pool.close().await;
    databases.close().await;
}
