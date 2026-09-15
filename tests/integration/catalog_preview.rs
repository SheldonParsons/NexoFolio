use nexofolio_application::CatalogPreviewService;
use nexofolio_contracts::*;
use nexofolio_infrastructure::{
    ChatCatalogGenerator, Postgres, PostgresCatalogPreviews, Unconfigured,
};
use nexofolio_knowledge::CatalogPreviewStore;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

fn uuid(id: impl ToString) -> Uuid {
    id.to_string().parse().unwrap()
}
async fn seed(sql: &sqlx::PgPool, p: Uuid, e: Uuid, user: Uuid, path: &str) -> InterfaceId {
    let interface = InterfaceId::new();
    let revision = Uuid::new_v4();
    let inbox = Uuid::new_v4();
    sqlx::query("INSERT INTO ingestion_inbox(id,project_id,actor_id,producer_id,source_type,record_id,batch_id,environment_id,identity_key,raw_record,status) VALUES($1,$2,$3,$4,'fixture',$5,$6,$7,$8,'{}','completed')")
        .bind(inbox).bind(p).bind(user).bind(Uuid::new_v4()).bind(Uuid::new_v4()).bind(Uuid::new_v4()).bind(e).bind(format!("GET {path}")).execute(sql).await.unwrap();
    sqlx::query("INSERT INTO interface_documents(id,project_id,identity_key,method,path) VALUES($1,$2,$3,'GET',$4)").bind(uuid(interface)).bind(p).bind(format!("GET {path}")).bind(path).execute(sql).await.unwrap();
    sqlx::query("INSERT INTO interface_observed_revisions(id,project_id,interface_id,environment_id,definition,definition_hash,origin_ingestion_id) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind(revision).bind(p).bind(uuid(interface)).bind(e).bind(json!({"method":"GET","path":path,"request":{"parameters":[]},"response":{"body":{"observed_schema":{"type":"object","properties":{"id":{"type":"number"}}}}}})).bind(vec![1u8]).bind(inbox).execute(sql).await.unwrap();
    sqlx::query("INSERT INTO interface_environment_current(interface_id,environment_id,current_revision_id) VALUES($1,$2,$3)").bind(uuid(interface)).bind(e).bind(revision).execute(sql).await.unwrap();
    interface
}
#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn preview_snapshot_model_contract_fencing_and_candidate_isolation() {
    let original = std::env::var("TEST_DATABASE_URL").unwrap();
    let admin = sqlx::PgPool::connect(&original).await.unwrap();
    let schema = format!("preview_{}", Uuid::new_v4().simple());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let separator = if original.contains('?') { "&" } else { "?" };
    let url = format!("{original}{separator}options=-csearch_path%3D{schema}");
    let db = Postgres::new(&Secret::new(&url), 5, Duration::from_secs(2)).unwrap();
    db.migrate().await.unwrap();
    let sql = sqlx::PgPool::connect(&url).await.unwrap();
    let project = ProjectId::new();
    let p = uuid(project);
    let e = Uuid::new_v4();
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO projects(id,instance,external_id,name,status,last_sync_generation) VALUES($1,'fixture','p','Project','doing',1)").bind(p).execute(&sql).await.unwrap();
    sqlx::query("INSERT INTO users(id,instance,external_id,account,display_name) VALUES($1,'fixture','u','u','User')").bind(user).execute(&sql).await.unwrap();
    sqlx::query("INSERT INTO environments(id,project_id,name) VALUES($1,$2,'开发环境')")
        .bind(e)
        .bind(p)
        .execute(&sql)
        .await
        .unwrap();
    let store = Arc::new(PostgresCatalogPreviews::new(db.clone()));
    assert!(store.create(project).await.is_err());
    let first = seed(&sql, p, e, user, "/users").await;
    let initial = store.create(project).await.unwrap();
    let second = seed(&sql, p, e, user, "/orders").await;
    assert_ne!(first, second);
    assert_eq!(
        store
            .read(initial.task_id)
            .await
            .unwrap()
            .snapshot
            .interfaces
            .len(),
        1
    );
    let unconfigured = CatalogPreviewService::standard(store.clone(), Arc::new(Unconfigured));
    assert!(matches!(
        unconfigured.run(initial.task_id).await,
        Err(Error::NotConfigured { .. })
    ));
    assert!(matches!(
        store.read(initial.task_id).await.unwrap().status,
        PreviewStatus::Pending
    ));

    // The actual HTTP adapter talks to a local protocol fixture, never a fake production path.
    let node = DirectoryId::new();
    let parent = DirectoryId::new();
    let candidate = json!({"nodes":[{"id":node,"parent":parent,"name":"用户管理","description":"用户接口"},{"id":parent,"parent":null,"name":"业务","description":"业务接口"}],"assignments":[{"interface_id":first,"directory_id":node,"reason":"/users路径"}]});
    let candidate_for_server = candidate.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let app=axum::Router::new().route("/v1/chat/completions",axum::routing::post(move |headers:axum::http::HeaderMap,axum::Json(body):axum::Json<Value>|{
        let candidate=candidate_for_server.clone();async move{
            assert_eq!(headers.get("authorization").unwrap(),"Bearer synthetic-model-key");
            assert_eq!(body["response_format"]["type"],"json_object");
            let input:Value=serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let content=if body["model"]=="omit-interface"{json!({"nodes":[],"assignments":[]}).to_string()}else{assert_eq!(input["interfaces"].as_array().unwrap().len(),1);candidate.to_string()};
            if body["model"]=="bad-response"{return axum::Json(json!({"choices":[{"finish_reason":"length","message":{"content":content}}]}));}
            axum::Json(json!({"choices":[{"finish_reason":"stop","message":{"content":content}}]}))
        }
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let service = |model: &str| {
        CatalogPreviewService::standard(
            store.clone(),
            Arc::new(
                ChatCatalogGenerator::new(&base, Secret::new("synthetic-model-key"), model.into())
                    .unwrap(),
            ),
        )
    };
    let result = service("fixture").run(initial.task_id).await.unwrap();
    assert!(matches!(result.status, PreviewStatus::Ready));
    assert!(result.review.as_ref().unwrap().structurally_valid);
    assert_eq!(result.snapshot_sha256, initial.snapshot_sha256);
    assert!(service("fixture").run(initial.task_id).await.is_err());
    let task_schema: Value = serde_json::from_str(include_str!(
        "../../contracts/catalog-preview/task.schema.json"
    ))
    .unwrap();
    assert!(
        jsonschema::validator_for(&task_schema)
            .unwrap()
            .is_valid(&serde_json::to_value(&result).unwrap())
    );
    let assignments: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM catalog_preview_assignments WHERE candidate_id=$1",
    )
    .bind(uuid(result.candidate_id))
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(assignments, 1);
    let docs: i64 = sqlx::query_scalar("SELECT count(*) FROM interface_environment_current")
        .fetch_one(&sql)
        .await
        .unwrap();
    assert_eq!(docs, 2);

    let rejected = service("omit-interface")
        .run(store.create(project).await.unwrap().task_id)
        .await
        .unwrap();
    assert!(matches!(rejected.status, PreviewStatus::Rejected));
    assert!(!rejected.review.unwrap().structurally_valid);
    let nodes: i64 =
        sqlx::query_scalar("SELECT count(*) FROM catalog_preview_nodes WHERE candidate_id=$1")
            .bind(uuid(rejected.candidate_id))
            .fetch_one(&sql)
            .await
            .unwrap();
    assert_eq!(nodes, 0);
    let failing = store.create(project).await.unwrap();
    // bad-response uses omit-interface output to keep fixture independent from snapshot size.
    let bad_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bad_base = format!("http://{}/v1", bad_listener.local_addr().unwrap());
    let bad_server = tokio::spawn(async move {
        axum::serve(
            bad_listener,
            axum::Router::new().route(
                "/v1/chat/completions",
                axum::routing::post(|| async {
                    axum::Json(
                        json!({"choices":[{"finish_reason":"length","message":{"content":"{}"}}]}),
                    )
                }),
            ),
        )
        .await
        .unwrap();
    });
    let failing_service = CatalogPreviewService::standard(
        store.clone(),
        Arc::new(
            ChatCatalogGenerator::new(&bad_base, Secret::new("synthetic"), "fixture".into())
                .unwrap(),
        ),
    );
    assert!(failing_service.run(failing.task_id).await.is_err());
    assert!(matches!(
        store.read(failing.task_id).await.unwrap().status,
        PreviewStatus::Failed
    ));
    assert_eq!(
        store
            .read(failing.task_id)
            .await
            .unwrap()
            .error_code
            .as_deref(),
        Some("MODEL_INVALID_RESULT")
    );
    let retry = service("omit-interface")
        .run(failing.task_id)
        .await
        .unwrap();
    assert_eq!(retry.generation, 2);

    let leased = store.create(project).await.unwrap();
    let info = GeneratorInfo {
        adapter: "test".into(),
        model: "test".into(),
        prompt_version: "test".into(),
        prompt_sha256: "fixture".into(),
    };
    let old = store.claim(leased.task_id, &info).await.unwrap();
    assert!(store.claim(leased.task_id, &info).await.is_err());
    sqlx::query("UPDATE catalog_preview_tasks SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1").bind(uuid(leased.task_id)).execute(&sql).await.unwrap();
    let fresh = store.claim(leased.task_id, &info).await.unwrap();
    assert_eq!(fresh.generation, old.generation + 1);
    assert!(
        store
            .complete(
                &old,
                &serde_json::from_value(candidate).unwrap(),
                result.review.as_ref().unwrap()
            )
            .await
            .is_err()
    );
    assert!(store.fail(&old, "TEST_STALE").await.is_err());
    store.fail(&fresh, "TEST_FINISHED").await.unwrap();
    assert_eq!(
        store
            .read(initial.task_id)
            .await
            .unwrap()
            .snapshot
            .interfaces
            .len(),
        1
    );
    // Public readers require a real internal session and current project access.
    use nexofolio_access::{ExternalIdentity, PlatformAccess};
    let access = Arc::new(
        nexofolio_infrastructure::PostgresAccess::new(db.clone(), &Secret::new("ab".repeat(32)))
            .unwrap(),
    );
    let session = access
        .normal_login(&ExternalIdentity {
            instance: "fixture".into(),
            external_id: "u".into(),
            account: "u".into(),
            display_name: "User".into(),
        })
        .await
        .unwrap();
    assert_eq!(uuid(session.user.id), user);
    sqlx::query("UPDATE users SET grants_synced=true WHERE id=$1")
        .bind(user)
        .execute(&sql)
        .await
        .unwrap();
    sqlx::query("INSERT INTO user_project_access(user_id,project_id) VALUES($1,$2)")
        .bind(user)
        .bind(p)
        .execute(&sql)
        .await
        .unwrap();
    let other = Uuid::new_v4();
    let denied = Uuid::new_v4();
    for (id, external) in [(other, "other"), (denied, "denied")] {
        sqlx::query("INSERT INTO projects(id,instance,external_id,name,status,last_sync_generation) VALUES($1,'fixture',$2,$2,'doing',1)").bind(id).bind(external).execute(&sql).await.unwrap();
    }
    sqlx::query("INSERT INTO user_project_access(user_id,project_id) VALUES($1,$2)")
        .bind(user)
        .bind(other)
        .execute(&sql)
        .await
        .unwrap();
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_base = format!("http://{}", http_listener.local_addr().unwrap());
    let routes = nexofolio_backend::http::catalog_preview::routes(
        nexofolio_backend::http::catalog_preview::CatalogPreviewHttp {
            reader: store.clone(),
            access: access.clone(),
        },
    );
    let official = Arc::new(nexofolio_infrastructure::PostgresOfficialCatalog::new(
        db.clone(),
    ));
    let publication = Arc::new(nexofolio_application::CatalogPublicationService::new(
        store.clone(),
        official.clone(),
    ));
    let routes = routes.merge(nexofolio_backend::http::official_catalog::routes(
        nexofolio_backend::http::official_catalog::OfficialCatalogHttp {
            access,
            reader: official,
            publication,
        },
    ));
    let http_server = tokio::spawn(async move {
        axum::serve(http_listener, routes).await.unwrap();
    });
    let client = reqwest::Client::new();
    let endpoint = format!("{http_base}/v1/projects/{project}/catalog-previews");
    assert_eq!(client.get(&endpoint).send().await.unwrap().status(), 401);
    let response = client
        .get(format!("{endpoint}?page=1&limit=1&status=ready"))
        .bearer_auth(session.token.expose())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let page: Value = response.json().await.unwrap();
    let page_schema: Value = serde_json::from_str(include_str!(
        "../../contracts/catalog-preview/page.schema.json"
    ))
    .unwrap();
    assert!(
        jsonschema::validator_for(&page_schema)
            .unwrap()
            .is_valid(&page)
    );
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["task_id"], initial.task_id.to_string());
    assert!(page["items"][0].get("snapshot").is_none());
    let response = client
        .get(format!("{endpoint}/{}", initial.task_id))
        .bearer_auth(session.token.expose())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let detail: Value = response.json().await.unwrap();
    let detail_schema: Value = serde_json::from_str(include_str!(
        "../../contracts/catalog-preview/detail.schema.json"
    ))
    .unwrap();
    assert!(
        jsonschema::validator_for(&detail_schema)
            .unwrap()
            .is_valid(&detail)
    );
    assert_eq!(detail["published"], false);
    assert_eq!(
        detail["task"]["snapshot"]["interfaces"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for (path, status) in [
        (
            format!(
                "{http_base}/v1/projects/{other}/catalog-previews/{}",
                initial.task_id
            ),
            404,
        ),
        (
            format!("{http_base}/v1/projects/{denied}/catalog-previews"),
            403,
        ),
        (
            format!(
                "{http_base}/v1/projects/{denied}/catalog-previews/{}",
                initial.task_id
            ),
            403,
        ),
        (format!("{endpoint}/{}", JobId::new()), 404),
        (format!("{endpoint}?page=0"), 400),
        (format!("{endpoint}?limit=101"), 400),
        (format!("{endpoint}?status=published"), 400),
        (format!("{endpoint}?environment_id={e}"), 400),
    ] {
        assert_eq!(
            client
                .get(path)
                .bearer_auth(session.token.expose())
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            status
        );
    }
    assert_eq!(
        client
            .post(&endpoint)
            .bearer_auth(session.token.expose())
            .send()
            .await
            .unwrap()
            .status(),
        405
    );
    sqlx::query("UPDATE users SET grants_synced=false WHERE id=$1")
        .bind(user)
        .execute(&sql)
        .await
        .unwrap();
    assert_eq!(
        client
            .get(&endpoint)
            .bearer_auth(session.token.expose())
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    sqlx::query("UPDATE users SET grants_synced=true WHERE id=$1")
        .bind(user)
        .execute(&sql)
        .await
        .unwrap();
    sqlx::query("DELETE FROM user_project_access WHERE user_id=$1 AND project_id=$2")
        .bind(user)
        .bind(p)
        .execute(&sql)
        .await
        .unwrap();
    assert_eq!(
        client
            .get(format!("{endpoint}/{}", initial.task_id))
            .bearer_auth(session.token.expose())
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let catalog = format!("{http_base}/v1/projects/{project}/catalog");
    assert_eq!(
        client
            .get(&catalog)
            .bearer_auth(session.token.expose())
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let denied_publish =
        json!({"task_id":initial.task_id,"expected_generation":0,"request_id":Uuid::new_v4()});
    write_catalog(
        &client,
        &format!("{catalog}/publish"),
        session.token.expose(),
        &denied_publish,
        403,
    )
    .await;
    let denied_restore =
        json!({"version_id":null,"expected_generation":0,"request_id":Uuid::new_v4()});
    write_catalog(
        &client,
        &format!("{catalog}/restore"),
        session.token.expose(),
        &denied_restore,
        403,
    )
    .await;
    sqlx::query("INSERT INTO user_project_access(user_id,project_id) VALUES($1,$2)")
        .bind(user)
        .bind(p)
        .execute(&sql)
        .await
        .unwrap();
    assert_eq!(client.get(&catalog).send().await.unwrap().status(), 401);
    let start: Value = client
        .get(&catalog)
        .bearer_auth(session.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(start["generation"], 0);
    assert_eq!(start["version_id"], Value::Null);
    assert_eq!(start["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(start["nodes"][0]["name"], "待分类");
    assert_eq!(start["nodes"][0]["locked"], true);
    assert_eq!(start["nodes"][0]["direct_interfaces"], 2);
    let system = start["unclassified_id"]
        .as_str()
        .unwrap()
        .parse::<Uuid>()
        .unwrap();
    assert!(
        sqlx::query("UPDATE project_catalogs SET unclassified_id=$2 WHERE project_id=$1")
            .bind(p)
            .bind(Uuid::new_v4())
            .execute(&sql)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM project_catalogs WHERE project_id=$1")
            .bind(p)
            .execute(&sql)
            .await
            .is_err()
    );
    let request =
        json!({"task_id":initial.task_id,"expected_generation":0,"request_id":Uuid::new_v4()});
    let first_activation = write_catalog(
        &client,
        &format!("{catalog}/publish"),
        session.token.expose(),
        &request,
        200,
    )
    .await;
    assert_eq!(first_activation["generation"], 1);
    let replay = write_catalog(
        &client,
        &format!("{catalog}/publish"),
        session.token.expose(),
        &request,
        200,
    )
    .await;
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["generation"], 1);
    let mut conflicting = request.clone();
    conflicting["expected_generation"] = json!(1);
    write_catalog(
        &client,
        &format!("{catalog}/publish"),
        session.token.expose(),
        &conflicting,
        409,
    )
    .await;
    let active: Value = client
        .get(&catalog)
        .bearer_auth(session.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(active["nodes"][0]["direct_interfaces"], 1);
    assert_eq!(active["unclassified_id"], start["unclassified_id"]);
    let official_schema: Value = serde_json::from_str(include_str!(
        "../../contracts/catalog-preview/official.schema.json"
    ))
    .unwrap();
    assert!(
        jsonschema::validator_for(&official_schema)
            .unwrap()
            .is_valid(&active)
    );
    let preview: Value = client
        .get(format!("{endpoint}/{}", initial.task_id))
        .bearer_auth(session.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(preview["published"], true);
    let cards: Value = client
        .get(format!(
            "{catalog}/interfaces?directory_id={system}&expected_generation=1"
        ))
        .bearer_auth(session.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cards["total"], 1);
    assert_eq!(cards["items"][0]["interface_id"], second.to_string());
    let third = seed(&sql, p, e, user, "/after-publication").await;
    let cards: Value = client
        .get(format!(
            "{catalog}/interfaces?directory_id={system}&expected_generation=1"
        ))
        .bearer_auth(session.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cards["total"], 2);
    assert!(
        cards["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["interface_id"] == third.to_string())
    );
    use nexofolio_knowledge::DocumentReader;
    let details = nexofolio_infrastructure::PostgresDocuments::new(db.clone())
        .detail(
            session.user.id,
            project,
            e.to_string().parse().unwrap(),
            first,
        )
        .await
        .unwrap();
    assert_eq!(details["classification"], "classified");
    assert_eq!(
        details["revision_id"],
        initial.snapshot.interfaces[0].environments[0]
            .revision_id
            .to_string()
    );
    let next_task = store.create(project).await.unwrap();
    let next_claim = store.claim(next_task.task_id, &info).await.unwrap();
    let mut next_candidate = result.candidate.clone().unwrap();
    let target = next_candidate.assignments[0].directory_id;
    next_candidate.assignments = next_task
        .snapshot
        .interfaces
        .iter()
        .map(|i| PreviewAssignment {
            interface_id: i.interface_id,
            directory_id: target,
            reason: "test route".into(),
        })
        .collect();
    let mut review = result.review.clone().unwrap();
    review.metrics.snapshot_interfaces = 3;
    review.metrics.assigned_interfaces = 3;
    review.metrics.logical_interfaces = Some(3);
    store
        .complete(&next_claim, &next_candidate, &review)
        .await
        .unwrap();
    let next_request =
        json!({"task_id":next_task.task_id,"expected_generation":1,"request_id":Uuid::new_v4()});
    // Force a failure after version/node insertion and verify full transaction rollback.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("ALTER TABLE catalog_version_assignments ADD CONSTRAINT reject_fixture_version CHECK (version_id <> '{}') NOT VALID",next_task.candidate_id))).execute(&sql).await.unwrap();
    write_catalog(
        &client,
        &format!("{catalog}/publish"),
        session.token.expose(),
        &next_request,
        503,
    )
    .await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog_versions")
        .fetch_one(&sql)
        .await
        .unwrap();
    assert_eq!(count, 1);
    sqlx::query("ALTER TABLE catalog_version_assignments DROP CONSTRAINT reject_fixture_version")
        .execute(&sql)
        .await
        .unwrap();
    write_catalog(
        &client,
        &format!("{catalog}/publish"),
        session.token.expose(),
        &next_request,
        200,
    )
    .await;
    let stale = json!({"version_id":null,"expected_generation":1,"request_id":Uuid::new_v4()});
    write_catalog(
        &client,
        &format!("{catalog}/restore"),
        session.token.expose(),
        &stale,
        409,
    )
    .await;
    let other_project = format!("{http_base}/v1/projects/{other}/catalog");
    let cross = json!({"version_id":initial.candidate_id,"expected_generation":0,"request_id":Uuid::new_v4()});
    write_catalog(
        &client,
        &format!("{other_project}/restore"),
        session.token.expose(),
        &cross,
        404,
    )
    .await;
    let cross_publish =
        json!({"task_id":initial.task_id,"expected_generation":0,"request_id":Uuid::new_v4()});
    write_catalog(
        &client,
        &format!("{other_project}/publish"),
        session.token.expose(),
        &cross_publish,
        404,
    )
    .await;
    let a = json!({"version_id":initial.candidate_id,"expected_generation":2,"request_id":Uuid::new_v4()});
    let b = json!({"version_id":null,"expected_generation":2,"request_id":Uuid::new_v4()});
    let (a, b) = tokio::join!(
        client
            .post(format!("{catalog}/restore"))
            .bearer_auth(session.token.expose())
            .json(&a)
            .send(),
        client
            .post(format!("{catalog}/restore"))
            .bearer_auth(session.token.expose())
            .json(&b)
            .send()
    );
    let mut statuses = vec![a.unwrap().status().as_u16(), b.unwrap().status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, vec![200, 409]);
    let now: Value = client
        .get(&catalog)
        .bearer_auth(session.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let reset = json!({"version_id":null,"expected_generation":now["generation"],"request_id":Uuid::new_v4()});
    write_catalog(
        &client,
        &format!("{catalog}/restore"),
        session.token.expose(),
        &reset,
        200,
    )
    .await;
    let reset_state: Value = client
        .get(&catalog)
        .bearer_auth(session.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reset_state["version_id"], Value::Null);
    assert_eq!(reset_state["nodes"][0]["direct_interfaces"], 3);
    assert_eq!(reset_state["unclassified_id"], start["unclassified_id"]);
    let old = json!({"version_id":initial.candidate_id,"expected_generation":reset_state["generation"],"request_id":Uuid::new_v4()});
    write_catalog(
        &client,
        &format!("{catalog}/restore"),
        session.token.expose(),
        &old,
        200,
    )
    .await;
    let restored: Value = client
        .get(&catalog)
        .bearer_auth(session.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(restored["nodes"][0]["direct_interfaces"], 2);
    let versions: Value = client
        .get(format!("{catalog}/versions"))
        .bearer_auth(session.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(versions["total"], 2);
    let poison = store.create(project).await.unwrap();
    let poison_claim = store.claim(poison.task_id, &info).await.unwrap();
    let mut poison_candidate = next_candidate.clone();
    poison_candidate.nodes[0].name = "待分类".into();
    store
        .complete(&poison_claim, &poison_candidate, &review)
        .await
        .unwrap();
    let poison_request = json!({"task_id":poison.task_id,"expected_generation":restored["generation"],"request_id":Uuid::new_v4()});
    write_catalog(
        &client,
        &format!("{catalog}/publish"),
        session.token.expose(),
        &poison_request,
        400,
    )
    .await;
    assert_eq!(
        client
            .get(format!("{catalog}/interfaces?expected_generation=0"))
            .bearer_auth(session.token.expose())
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    // Original state/revisions survive all publications and restorations.
    let revisions: i64 = sqlx::query_scalar("SELECT count(*) FROM interface_observed_revisions")
        .fetch_one(&sql)
        .await
        .unwrap();
    assert_eq!(revisions, 3);
    http_server.abort();
    server.abort();
    bad_server.abort();
    sql.close().await;
    db.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

#[tokio::test]
async fn model_adapter_rejects_truncation_unknown_fields_and_http_errors() {
    use nexofolio_contracts::CatalogSnapshot;
    // Exercise through the application with a protocol fixture in the DB test; here validate
    // adapter wire handling via a tiny generator-independent response server.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let app=axum::Router::new().route("/v1/chat/completions",axum::routing::post(|axum::Json(request):axum::Json<Value>|async move{
        use axum::response::IntoResponse;
        match request["model"].as_str().unwrap(){
            "http-error"=>(axum::http::StatusCode::BAD_GATEWAY,"private provider body").into_response(),
            "truncated"=>axum::Json(json!({"choices":[{"finish_reason":"length","message":{"content":"{}"}}]})).into_response(),
            "unknown-field"=>axum::Json(json!({"choices":[{"finish_reason":"stop","message":{"content":"{\"nodes\":[],\"assignments\":[],\"publish\":true}"}}]})).into_response(),
            _=>axum::Json(json!({"choices":[{"finish_reason":"stop","message":{"content":"not-json"}}]})).into_response(),
        }
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    // Local trait re-export is unnecessary: use the concrete adapter through a bounded store
    // facade below to retain the backend dependency whitelist.
    let _snapshot = CatalogSnapshot {
        project_id: ProjectId::new(),
        project_name: "fixture".into(),
        interfaces: vec![],
    };
    for model in ["http-error", "truncated", "unknown-field", "not-json"] {
        let generator =
            ChatCatalogGenerator::new(&base, Secret::new("secret-fixture"), model.into()).unwrap();
        let store = Arc::new(SingleTaskStore::new());
        let service = CatalogPreviewService::standard(store.clone(), Arc::new(generator));
        let result = service.run(store.task.task_id).await;
        let message = result.unwrap_err().to_string();
        assert!(!message.contains("private provider body"));
        assert!(!message.contains("secret-fixture"));
        assert!(store.failed.load(std::sync::atomic::Ordering::SeqCst));
    }
    server.abort();
}
struct SingleTaskStore {
    task: PreviewTask,
    failed: std::sync::atomic::AtomicBool,
}
impl SingleTaskStore {
    fn new() -> Self {
        Self {
            task: PreviewTask {
                task_id: JobId::new(),
                candidate_id: CatalogVersion::new(),
                contract_version: CATALOG_PREVIEW_VERSION.into(),
                status: PreviewStatus::Pending,
                snapshot_at: "fixture".into(),
                snapshot_sha256: "fixture".into(),
                snapshot: CatalogSnapshot {
                    project_id: ProjectId::new(),
                    project_name: "fixture".into(),
                    interfaces: vec![],
                },
                generation: 1,
                generator: None,
                error_code: None,
                candidate: None,
                review: None,
            },
            failed: std::sync::atomic::AtomicBool::new(false),
        }
    }
}
#[async_trait::async_trait]
impl CatalogPreviewStore for SingleTaskStore {
    async fn create(&self, _: ProjectId) -> Result<PreviewTask> {
        Ok(self.task.clone())
    }
    async fn read(&self, _: JobId) -> Result<PreviewTask> {
        Ok(self.task.clone())
    }
    async fn claim(&self, _: JobId, _: &GeneratorInfo) -> Result<PreviewTask> {
        Ok(self.task.clone())
    }
    async fn complete(
        &self,
        _: &PreviewTask,
        _: &DirectoryCandidate,
        _: &PreviewReview,
    ) -> Result<()> {
        panic!("invalid response must not complete")
    }
    async fn fail(&self, _: &PreviewTask, _: &'static str) -> Result<()> {
        self.failed.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

async fn write_catalog(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    body: &Value,
    status: u16,
) -> Value {
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), status);
    response.json().await.unwrap()
}
