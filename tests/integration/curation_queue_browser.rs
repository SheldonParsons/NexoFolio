//! Opt-in browser fixture: private database, actual API and fixed localhost upstream only.
#[allow(dead_code)]
#[path = "curation_restart/fixture.rs"]
mod fixture;
use nexofolio_common::{EnvironmentId, ProjectId};
use nexofolio_contracts::{endpoint::EndpointReader, observation::*, testing::sample_exchange};
use nexofolio_observe::Observe;
use serde_json::{Value, json};

#[tokio::test]
#[ignore = "manual isolated browser acceptance; requires QUEUE_BROWSER_FIXTURE under /tmp/nexofolio-queue-fixes"]
async fn serve_browser_fixture() {
    let path = std::path::PathBuf::from(
        std::env::var("QUEUE_BROWSER_FIXTURE").expect("private output path"),
    );
    assert!(path.starts_with("/tmp/nexofolio-queue-fixes"));
    let db_url = fixture::database_url();
    let upstream = fixture::Upstream::start().await;
    let db = upstream.databases(&db_url).await;
    let mut api = fixture::ApiProcess::start(&db_url, &upstream).await;
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
    let environment = client
        .post(format!("{}/v1/projects/{project}/environments", api.url))
        .bearer_auth(token)
        .json(&json!({"name":"队列隔离验收"}))
        .send()
        .await
        .unwrap();
    assert!(
        environment.status().is_success(),
        "fixture environment creation failed"
    );
    let environment: Value = environment.json().await.unwrap();
    let env: EnvironmentId = environment["id"].as_str().unwrap().parse().unwrap();
    let observe = Observe::new(db.observe.clone());
    for name in ["queue-one", "queue-two"] {
        let mut observation = sample_exchange(project, env);
        observation.observed_at = chrono::Utc::now();
        let Fact::Exchange(exchange) = &mut observation.fact else {
            unreachable!()
        };
        exchange.request.method = "POST".into();
        exchange.request.url = format!("https://synthetic.example/{name}");
        exchange.request.headers = None;
        exchange.response = None;
        exchange.request.body = Body::Full {
            media_type: Some("application/json".into()),
            content: Content::Text(json!({"f1":"a","f2":"b","f3":"c"}).to_string()),
        };
        observe.accept(vec![observation]).await.unwrap();
    }
    let endpoints = observe.list(project).await.unwrap();
    std::fs::write(
        &path,
        serde_json::to_vec(
            &json!({"api":api.url,"project":project,"environment":env,"endpoints":endpoints}),
        )
        .unwrap(),
    )
    .unwrap();
    // The browser logs in itself. No session credentials are written to the fixture manifest.
    let stop = path.with_extension("stop");
    tokio::time::timeout(std::time::Duration::from_secs(3600), async {
        while !stop.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap();
    api.stop().await;
    db.close().await;
}
