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
    let id: uuid::Uuid = progress["candidate_id"].as_str().unwrap().parse().unwrap();
    let knowledge = Knowledge::new(databases.knowledge.clone(), observe);
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
    tx.put_candidate(&stopped).await.unwrap();
    tx.commit().await.unwrap();
    upstream.model.block.store(false, Ordering::SeqCst);
    // Two fresh API instances use the real startup path, without a resume HTTP call.
    let (mut next, mut competing) = tokio::join!(
        ApiProcess::start(&db, &upstream),
        ApiProcess::start(&db, &upstream)
    );
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
    next.stop().await;
    competing.stop().await;
    databases.close().await;
}
