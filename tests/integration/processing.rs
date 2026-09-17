use nexofolio_access::{
    ExternalIdentity, ExternalProject, PlatformAccess, ProjectSnapshot, SessionPrincipal,
};
use nexofolio_application::ProcessingService;
use nexofolio_backend::{
    http,
    wiring::{Config, build_access},
};
use nexofolio_contracts::{InterfaceId, Secret};
use nexofolio_infrastructure::{Postgres, PostgresAccess, PostgresDocuments, Unconfigured};
use nexofolio_knowledge::{
    AssessmentReader, DocumentQuery, DocumentReader, ObservationProcessor, extract_observed,
};
use serde_json::{Value, json};
use sqlx::Row;
use std::{sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn assert_contract(name: &str, value: &Value) {
    let mut schema: Value = serde_json::from_str(include_str!(
        "../../contracts/documents/responses.schema.json"
    ))
    .unwrap();
    schema["$ref"] = json!(format!("#/$defs/{name}"));
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();
    assert!(
        validator.is_valid(value),
        "response failed document contract {name}"
    );
}
async fn insert(
    pool: &sqlx::PgPool,
    project: Uuid,
    env: Uuid,
    actor: Uuid,
    path: &str,
    content: &str,
) -> Uuid {
    let mut raw: Value = serde_json::from_str(include_str!(
        "../../contracts/ingestion/fixtures/http-batch.json"
    ))
    .unwrap();
    raw = raw["records"][0].clone();
    raw["record_id"] = json!(Uuid::new_v4());
    raw["payload"]["request"]["url"] = json!(format!("https://api.example.test{path}"));
    raw["payload"]["response"]["body"]["content"] = json!(content);
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO ingestion_inbox(id,project_id,actor_id,producer_id,source_type,record_id,batch_id,environment_id,identity_key,raw_record) VALUES($1,$2,$3,$4,'fixture',$5,$6,$7,$8,$9)").bind(id).bind(project).bind(actor).bind(Uuid::new_v4()).bind(Uuid::new_v4()).bind(Uuid::new_v4()).bind(env).bind(format!("GET {path}")).bind(raw).execute(pool).await.unwrap();
    id
}
#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn documents_are_idempotent_environment_scoped_fenced_and_queryable() {
    let original_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let admin = sqlx::PgPool::connect(&original_url).await.unwrap();
    // Identifier consists only of a fixed prefix and generated UUID hex, never user input.
    let schema = format!("processing_{}", Uuid::new_v4().simple());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let separator = if original_url.contains('?') { "&" } else { "?" };
    let url = format!("{original_url}{separator}options=-csearch_path%3D{schema}");
    let db = Postgres::new(&Secret::new(&url), 10, Duration::from_secs(2)).unwrap();
    db.migrate().await.unwrap();
    let sql = sqlx::PgPool::connect(&url).await.unwrap();
    let instance = format!("http://127.0.0.1:1/processing-{}", Uuid::new_v4());
    let key = "ce".repeat(32);
    let access = PostgresAccess::new(db.clone(), &Secret::new(&key)).unwrap();
    let session = access
        .normal_login(&ExternalIdentity {
            instance: instance.clone(),
            external_id: "user".into(),
            account: "user".into(),
            display_name: "User".into(),
        })
        .await
        .unwrap();
    let principal = SessionPrincipal {
        user_id: session.user.id,
        instance: instance.clone(),
    };
    access
        .apply_snapshot(
            &principal,
            access.next_sync_generation().await.unwrap(),
            ProjectSnapshot {
                projects: vec![
                    ExternalProject {
                        instance: instance.clone(),
                        external_id: "p".into(),
                        name: "Test".into(),
                        state: "doing".into(),
                    },
                    ExternalProject {
                        instance: instance.clone(),
                        external_id: "denied".into(),
                        name: "Denied".into(),
                        state: "doing".into(),
                    },
                ],
                visible_project_ids: vec!["p".into()],
            },
        )
        .await
        .unwrap();
    let projects = access
        .list_projects(&principal, 1, 100)
        .await
        .unwrap()
        .items;
    let project = projects.iter().find(|p| p.can_access).unwrap().project_id;
    let denied = projects.iter().find(|p| !p.can_access).unwrap().project_id;
    let dev = access
        .create_environment(&principal, project, "开发环境")
        .await
        .unwrap();
    let prod = access
        .create_environment(&principal, project, "生产环境")
        .await
        .unwrap();
    let p: Uuid = project.to_string().parse().unwrap();
    let e: Uuid = dev.id.to_string().parse().unwrap();
    let actor: Uuid = session.user.id.to_string().parse().unwrap();
    let first = insert(
        &sql,
        p,
        e,
        actor,
        "/orders",
        "{\"id\":1,\"name\":\"first\"}",
    )
    .await;
    let second = insert(
        &sql,
        p,
        e,
        actor,
        "/orders",
        "{\"id\":2,\"name\":\"changed-value\"}",
    )
    .await;
    let changed = insert(
        &sql,
        p,
        e,
        actor,
        "/orders",
        "{\"id\":2,\"name\":\"new\",\"added\":true}",
    )
    .await;
    let repeated = insert(
        &sql,
        p,
        e,
        actor,
        "/orders",
        "{\"id\":3,\"name\":\"new-value\",\"added\":false}",
    )
    .await;
    let in_prod = insert(
        &sql,
        p,
        prod.id.to_string().parse().unwrap(),
        actor,
        "/orders",
        "{\"different_environment\":true}",
    )
    .await;
    let processor = Arc::new(PostgresDocuments::new(db.clone()));
    let service = ProcessingService::new(processor.clone());
    let a = processor.claim().await.unwrap().unwrap();
    assert_eq!(a.ingestion_id, first);
    // Another worker may claim another environment, but cannot overtake the same-key first item.
    let b = processor.claim().await.unwrap().unwrap();
    assert_eq!(b.ingestion_id, in_prod);
    processor
        .finish(&b, extract_observed(&b.raw).unwrap())
        .await
        .unwrap();
    let result = processor
        .finish(&a, extract_observed(&a.raw).unwrap())
        .await
        .unwrap();
    let interface = result.interface_id;
    assert!(
        processor
            .finish(&a, extract_observed(&a.raw).unwrap())
            .await
            .is_err()
    );
    assert_eq!(
        service.process_one().await.unwrap().unwrap().ingestion_id,
        second
    );
    assert_eq!(
        service.process_one().await.unwrap().unwrap().ingestion_id,
        changed
    );
    assert_eq!(
        service.process_one().await.unwrap().unwrap().ingestion_id,
        repeated
    );
    assert!(service.process_one().await.unwrap().is_none());
    let dev_detail = processor
        .detail(session.user.id, project, dev.id, interface)
        .await
        .unwrap();
    let prod_detail = processor
        .detail(session.user.id, project, prod.id, interface)
        .await
        .unwrap();
    assert_contract("InterfaceDetail", &dev_detail);
    assert_contract("InterfaceDetail", &prod_detail);
    assert_ne!(dev_detail["revision_id"], prod_detail["revision_id"]);
    assert_eq!(dev_detail["classification"], "unclassified");
    assert_eq!(dev_detail["pending_difference_count"], 1);
    assert!(
        dev_detail["definition"]["response"]["body"]["observed_schema"]["properties"]
            .get("added")
            .is_none()
    );
    let obs = processor
        .observation(session.user.id, project, changed)
        .await
        .unwrap();
    assert_contract("ObservationDetail", &obs);
    assert_eq!(obs["status"], "completed");
    assert_eq!(obs["outcome"], "difference_recorded");
    assert!(
        obs["proposed_definition"]["response"]["body"]["observed_schema"]["properties"]
            .get("added")
            .is_some()
    );
    assert!(
        obs["raw_record"]["payload"]["request"]["headers"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h[0] == "Authorization")
    );
    assert_eq!(
        processor
            .observation(session.user.id, project, repeated)
            .await
            .unwrap()["difference_id"],
        obs["difference_id"]
    );
    assert_eq!(
        processor
            .list(
                session.user.id,
                project,
                DocumentQuery {
                    environment_id: dev.id,
                    page: 1,
                    limit: 1,
                    query: Some("order".into())
                }
            )
            .await
            .unwrap()["total"],
        1
    );
    assert_eq!(
        processor
            .observations(session.user.id, project, dev.id, interface, 1, 2)
            .await
            .unwrap()["total"],
        4
    );
    // A killed worker's expired lease is reclaimed with a larger fencing generation.
    let recover = insert(&sql, p, e, actor, "/recover", "{}").await;
    let old = processor.claim().await.unwrap().unwrap();
    assert_eq!(old.ingestion_id, recover);
    sqlx::query(
        "UPDATE ingestion_inbox SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(recover)
    .execute(&sql)
    .await
    .unwrap();
    let replacement = processor.claim().await.unwrap().unwrap();
    assert!(replacement.generation > old.generation);
    assert!(
        processor
            .finish(&old, extract_observed(&old.raw).unwrap())
            .await
            .is_err()
    );
    assert!(processor.fail(&old, "late error").await.is_err());
    processor
        .finish(&replacement, extract_observed(&replacement.raw).unwrap())
        .await
        .unwrap();
    // Transaction failure cannot leave a document behind while the inbox remains unprocessed.
    let rollback = insert(&sql, p, e, actor, "/rollback", "{}").await;
    let c = processor.claim().await.unwrap().unwrap();
    assert_eq!(c.ingestion_id, rollback);
    sqlx::query("ALTER TABLE interface_observations ADD CONSTRAINT test_reject_outcome CHECK(outcome <> 'created') NOT VALID").execute(&sql).await.unwrap();
    assert!(
        processor
            .finish(&c, extract_observed(&c.raw).unwrap())
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM interface_documents WHERE project_id=$1 AND path='/rollback'",
    )
    .bind(p)
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(count, 0);
    sqlx::query("ALTER TABLE interface_observations DROP CONSTRAINT test_reject_outcome")
        .execute(&sql)
        .await
        .unwrap();
    processor
        .finish(&c, extract_observed(&c.raw).unwrap())
        .await
        .unwrap();
    // Bounded retries, blocked same-key successor, explicit retry, then recovery.
    let bad = insert(&sql, p, e, actor, "/bad", "{}").await;
    sqlx::query("UPDATE ingestion_inbox SET raw_record='{}'::jsonb WHERE id=$1")
        .bind(bad)
        .execute(&sql)
        .await
        .unwrap();
    let after_bad = insert(&sql, p, e, actor, "/bad", "{}").await;
    for _ in 0..5 {
        assert!(service.process_one().await.is_err());
        sqlx::query(
            "UPDATE ingestion_inbox SET retry_at=clock_timestamp()-interval '1 second' WHERE id=$1",
        )
        .bind(bad)
        .execute(&sql)
        .await
        .unwrap();
    }
    assert!(service.process_one().await.unwrap().is_none());
    let bad_observation = processor
        .observation(session.user.id, project, bad)
        .await
        .unwrap();
    assert_eq!(bad_observation["status"], "failed");
    assert_eq!(bad_observation["attempts"], 5);
    sqlx::query("UPDATE ingestion_inbox SET raw_record=(SELECT raw_record FROM ingestion_inbox WHERE id=$2) WHERE id=$1").bind(bad).bind(after_bad).execute(&sql).await.unwrap();
    processor.retry_failed(bad).await.unwrap();
    assert!(service.process_one().await.unwrap().is_some());
    assert!(service.process_one().await.unwrap().is_some());
    // Actual HTTP queries enforce project/environment access and omit payloads from lists.
    assert_contract(
        "ObservationPage",
        &processor
            .observations(session.user.id, project, dev.id, interface, 1, 2)
            .await
            .unwrap(),
    );
    let config = Config::from_lookup(|k| match k {
        "DATABASE_URL" => Some(url.clone()),
        "NEXOFOLIO_ZENTAO_BASE_URL" => Some(instance.clone()),
        "NEXOFOLIO_SESSION_KEY" => Some(key.clone()),
        _ => None,
    })
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let stop = CancellationToken::new();
    let app = http::router_with_access(
        &config,
        Arc::new(db.clone()),
        Arc::new(Unconfigured),
        stop.clone(),
        build_access(&config, db.clone()).unwrap(),
    );
    let handle = tokio::spawn(http::serve(
        listener,
        app,
        stop.clone(),
        Duration::from_secs(2),
    ));
    let client = reqwest::Client::new();
    let list_url = format!(
        "{base}/v1/projects/{project}/interfaces?environment_id={}",
        dev.id
    );
    assert_eq!(client.get(&list_url).send().await.unwrap().status(), 401);
    let response = client
        .get(&list_url)
        .bearer_auth(session.token.expose())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let items: Value = response.json().await.unwrap();
    assert_contract("InterfacePage", &items);
    assert!(
        items["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v.get("raw_record").is_none())
    );
    for path in [
        format!(
            "/v1/projects/{project}/interfaces/{interface}?environment_id={}",
            dev.id
        ),
        format!(
            "/v1/projects/{project}/interfaces/{interface}/observations?environment_id={}",
            dev.id
        ),
        format!("/v1/projects/{project}/observations/{changed}"),
    ] {
        assert_eq!(
            client
                .get(format!("{base}{path}"))
                .bearer_auth(session.token.expose())
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
    }
    assert_eq!(
        client
            .get(format!(
                "{base}/v1/projects/{denied}/interfaces?environment_id={}",
                dev.id
            ))
            .bearer_auth(session.token.expose())
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .get(format!(
                "{base}/v1/projects/{project}/interfaces/{}?environment_id={}",
                InterfaceId::new(),
                dev.id
            ))
            .bearer_auth(session.token.expose())
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let revisions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM interface_observed_revisions WHERE interface_id=$1",
    )
    .bind(uuid(interface))
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(revisions, 2);
    let status: String = sqlx::query("SELECT status FROM ingestion_inbox WHERE id=$1")
        .bind(first)
        .fetch_one(&sql)
        .await
        .unwrap()
        .get("status");
    assert_eq!(status, "completed");
    // End-to-end: real batch HTTP admission -> durable worker -> document detail HTTP.
    let worker_stop = CancellationToken::new();
    let worker = tokio::spawn(nexofolio_backend::wiring::run_processing_worker(
        ProcessingService::new(processor.clone()),
        worker_stop.clone(),
    ));
    let mut batch: Value = serde_json::from_str(include_str!(
        "../../contracts/ingestion/fixtures/http-batch.json"
    ))
    .unwrap();
    batch["project_id"] = json!(project);
    batch["environment"] = json!({"id":dev.id});
    batch["source"]["instance_id"] = json!(Uuid::new_v4());
    batch["records"][0]["record_id"] = json!(Uuid::new_v4());
    batch["records"][0]["payload"]["request"]["url"] = json!("https://api.example.test/http-e2e");
    let accepted = client
        .post(format!("{base}/v1/ingestion/batches"))
        .bearer_auth(session.token.expose())
        .json(&batch)
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), 200);
    let accepted: Value = accepted.json().await.unwrap();
    assert_eq!(accepted["results"][0]["status"], "accepted");
    let ingestion = accepted["results"][0]["ingestion_id"].as_str().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let result: Value = client
            .get(format!(
                "{base}/v1/projects/{project}/observations/{ingestion}"
            ))
            .bearer_auth(session.token.expose())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if result["status"] == "completed" {
            assert_eq!(result["outcome"], "created");
            let id = result["interface_id"].as_str().unwrap();
            let detail: Value = client
                .get(format!(
                    "{base}/v1/projects/{project}/interfaces/{id}?environment_id={}",
                    dev.id
                ))
                .bearer_auth(session.token.expose())
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(detail["path"], "/http-e2e");
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut route_batch = batch.clone();
    let mut admitted = Vec::new();
    for (value, changed, expected) in [
        (123, false, "accepted"),
        (456, false, "ignored"),
        (789, true, "accepted"),
    ] {
        route_batch["records"][0]["record_id"] = json!(Uuid::new_v4());
        route_batch["records"][0]["payload"]["request"]["url"] =
            json!(format!("https://api.example.test/numeric-users/{value}"));
        if changed {
            route_batch["records"][0]["payload"]["response"]["body"]["content"] =
                json!(r#"{"items":[],"added":true}"#);
        }
        let receipt: Value = client
            .post(format!("{base}/v1/ingestion/batches"))
            .bearer_auth(session.token.expose())
            .json(&route_batch)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(receipt["results"][0]["status"], expected);
        if expected == "accepted" {
            admitted.push(
                receipt["results"][0]["ingestion_id"]
                    .as_str()
                    .unwrap()
                    .parse::<Uuid>()
                    .unwrap(),
            );
        }
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut route_id = None;
    for (index, id) in admitted.iter().enumerate() {
        loop {
            let obs = processor
                .observation(session.user.id, project, *id)
                .await
                .unwrap();
            if obs["status"] == "completed" {
                let id: InterfaceId = obs["interface_id"].as_str().unwrap().parse().unwrap();
                if index == 0 {
                    route_id = Some(id);
                } else {
                    assert_eq!(Some(id), route_id);
                    assert_eq!(obs["outcome"], "difference_recorded");
                }
                let detail = processor
                    .detail(session.user.id, project, dev.id, id)
                    .await
                    .unwrap();
                assert_eq!(detail["path"], "/numeric-users/{param1}");
                assert_eq!(
                    detail["definition"]["request"]["parameters"][0]["in"],
                    "path"
                );
                assert_contract("InterfaceDetail", &detail);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    worker_stop.cancel();
    tokio::time::timeout(Duration::from_secs(2), worker)
        .await
        .unwrap()
        .unwrap();
    // Older queued observations still receive the same directional rule in processing.
    for (path, bodies, outcomes) in [
        (
            "/array-full-first",
            [r#"{"items":[{"id":1}]}"#, r#"{"items":[]}"#],
            ["created", "unchanged"],
        ),
        (
            "/array-empty-first",
            [r#"{"items":[]}"#, r#"{"items":[{"id":1}]}"#],
            ["created", "difference_recorded"],
        ),
        (
            "/array-sibling-change",
            [r#"{"items":[{"id":1}]}"#, r#"{"items":[],"added":true}"#],
            ["created", "difference_recorded"],
        ),
    ] {
        let mut baseline = Value::Null;
        for (body, outcome) in bodies.into_iter().zip(outcomes) {
            let id = insert(&sql, p, e, actor, path, body).await;
            let processed = service.process_one().await.unwrap().unwrap();
            assert_eq!(processed.ingestion_id, id);
            let obs = processor
                .observation(session.user.id, project, id)
                .await
                .unwrap();
            assert_eq!(obs["outcome"], outcome);
            assert_eq!(
                obs["raw_record"]["payload"]["response"]["body"]["content"],
                body
            );
            let detail = processor
                .detail(session.user.id, project, dev.id, processed.interface_id)
                .await
                .unwrap();
            if outcome == "created" {
                baseline = detail["definition"].clone();
            } else {
                assert_eq!(detail["definition"], baseline);
            }
            if outcome == "unchanged" {
                assert_eq!(detail["pending_difference_count"], 0);
            }
            let material = processor
                .assessment(session.user.id, project, id)
                .await
                .unwrap();
            assert_eq!(
                material.assessment.as_ref().unwrap().initial,
                outcome == "created"
            );
            assert_eq!(material.base_revision_id.is_none(), outcome == "created");
            if path == "/array-empty-first" && outcome != "created" {
                assert_eq!(
                    material.assessment.as_ref().unwrap().categories,
                    [nexofolio_contracts::AssessmentKind::Enrichment]
                );
                assert!(material.incoming_definition.is_some());
            }
            let endpoint = format!("{base}/v1/projects/{project}/observations/{id}/assessment");
            assert_eq!(client.get(&endpoint).send().await.unwrap().status(), 401);
            let response = client
                .get(&endpoint)
                .bearer_auth(session.token.expose())
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let value: Value = response.json().await.unwrap();
            let schema: Value = serde_json::from_str(include_str!(
                "../../contracts/documents/assessment.schema.json"
            ))
            .unwrap();
            assert!(
                jsonschema::options()
                    .should_validate_formats(true)
                    .build(&schema)
                    .unwrap()
                    .is_valid(&value)
            );
            assert_eq!(
                client
                    .get(format!(
                        "{base}/v1/projects/{denied}/observations/{id}/assessment"
                    ))
                    .bearer_auth(session.token.expose())
                    .send()
                    .await
                    .unwrap()
                    .status(),
                403
            );
            let page = processor
                .assessments(
                    session.user.id,
                    project,
                    processed.interface_id,
                    DocumentQuery {
                        environment_id: dev.id,
                        page: 1,
                        limit: 20,
                        query: None,
                    },
                )
                .await
                .unwrap();
            assert!(page.items.iter().any(|r| r.ingestion_id == id));
            let page_response=client.get(format!("{base}/v1/projects/{project}/interfaces/{}/assessments?environment_id={}&page=1&limit=20",processed.interface_id,dev.id)).bearer_auth(session.token.expose()).send().await.unwrap();
            assert_eq!(page_response.status(), 200);
            let page_value: Value = page_response.json().await.unwrap();
            let page_schema: Value = serde_json::from_str(include_str!(
                "../../contracts/documents/assessment-page.schema.json"
            ))
            .unwrap();
            assert!(
                jsonschema::options()
                    .should_validate_formats(true)
                    .build(&page_schema)
                    .unwrap()
                    .is_valid(&page_value)
            );

            // The read API distinguishes historical/unassessed from a newly computed duplicate.
            sqlx::query("UPDATE interface_observations SET assessment=NULL,extractor_version=NULL WHERE ingestion_id=$1").bind(id).execute(&sql).await.unwrap();
            assert!(
                processor
                    .assessment(session.user.id, project, id)
                    .await
                    .unwrap()
                    .assessment
                    .is_none()
            );
        }
    }
    stop.cancel();
    handle.await.unwrap().unwrap();
    sql.close().await;
    db.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
fn uuid(id: impl ToString) -> Uuid {
    id.to_string().parse().unwrap()
}
