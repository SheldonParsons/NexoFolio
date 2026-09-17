use nexofolio_contracts::*;
use nexofolio_infrastructure::{ChatMaintenanceModel, Postgres, PostgresCatalogPreviews};
use nexofolio_rebuild::{DirectoryReviewer, MaintenanceModel, StructuralDirectoryReviewer};
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
async fn historical_candidates_remain_readable_and_publication_isolated() {
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
    let first = seed(&sql, p, e, user, "/users").await;
    let initial = historical_snapshot(&sql, &store, project).await;
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
    let node = DirectoryId::new();
    let parent = DirectoryId::new();
    let candidate:DirectoryCandidate=serde_json::from_value(json!({"nodes":[{"id":node,"parent":parent,"name":"用户管理","description":"用户接口"},{"id":parent,"parent":null,"name":"业务","description":"业务接口"}],"assignments":[{"interface_id":first,"directory_id":node,"reason":"historical fixture"}]})).unwrap();
    let review = StructuralDirectoryReviewer.review(&initial.snapshot, &candidate);
    save_historical_candidate(&sql, &initial, &candidate, &review).await;
    let result = store.read(initial.task_id).await.unwrap();
    assert!(matches!(result.status, PreviewStatus::Ready));
    assert_eq!(result.snapshot_sha256, initial.snapshot_sha256);
    let task_schema: Value = serde_json::from_str(include_str!(
        "../../contracts/catalog-preview/task.schema.json"
    ))
    .unwrap();
    assert!(
        jsonschema::validator_for(&task_schema)
            .unwrap()
            .is_valid(&serde_json::to_value(&result).unwrap())
    );
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
    // Applying the retirement migration to historical data stops only unfinished tasks.
    let unfinished = historical_snapshot(&sql, &store, project).await;
    sqlx::raw_sql(include_str!(
        "../../migrations/202609150006_retire_directory_generation.sql"
    ))
    .execute(&sql)
    .await
    .unwrap();
    let retired = store.read(unfinished.task_id).await.unwrap();
    assert!(matches!(retired.status, PreviewStatus::Failed));
    assert_eq!(
        retired.error_code.as_deref(),
        Some("LEGACY_GENERATION_RETIRED")
    );
    assert!(matches!(
        store.read(initial.task_id).await.unwrap().status,
        PreviewStatus::Ready
    ));
    assert_eq!(retired.snapshot_sha256, unfinished.snapshot_sha256);
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
    assert_eq!(
        client
            .post(&endpoint)
            .bearer_auth(session.token.expose())
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        405,
        "historical readers must not accept generation requests"
    );
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
    let next_task = historical_snapshot(&sql, &store, project).await;
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
    save_historical_candidate(&sql, &next_task, &next_candidate, &review).await;
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
    let poison = historical_snapshot(&sql, &store, project).await;
    let mut poison_candidate = next_candidate.clone();
    poison_candidate.nodes[0].name = "待分类".into();
    save_historical_candidate(&sql, &poison, &poison_candidate, &review).await;
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
    sql.close().await;
    db.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

#[tokio::test]
async fn maintenance_model_rejects_truncation_unknown_contract_fields_and_http_errors() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let app=axum::Router::new().route("/v1/chat/completions",axum::routing::post(|axum::Json(request):axum::Json<Value>|async move{
        use axum::response::IntoResponse;
        match request["model"].as_str().unwrap(){
            "http-error"=>(axum::http::StatusCode::BAD_GATEWAY,"private provider body").into_response(),
            "truncated"=>axum::Json(json!({"choices":[{"finish_reason":"length","message":{"content":"{}"}}]})).into_response(),
            "unknown-field"=>axum::Json(json!({"choices":[{"finish_reason":"stop","message":{"content":"{\"type\":\"summary\",\"summary\":\"x\",\"publish\":true}"}}]})).into_response(),
            _=>axum::Json(json!({"choices":[{"finish_reason":"stop","message":{"content":"not-json"}}]})).into_response(),
        }
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    for name in ["http-error", "truncated", "unknown-field", "not-json"] {
        let model =
            ChatMaintenanceModel::new(&base, Secret::new("secret-fixture"), name.into(), false)
                .unwrap();
        let request = model.prepare("plan", json!({}), 2048).unwrap();
        match model.invoke(&request).await {
            Ok(value) => assert!(serde_json::from_value::<MaintenanceReply>(value).is_err()),
            Err(error) => {
                let message = error.to_string();
                assert!(!message.contains("secret-fixture"));
                assert!(!message.contains("private provider body"));
            }
        }
    }
    server.abort();
}

#[test]
fn retired_generation_commands_are_not_executable() {
    let binary = env!("CARGO_BIN_EXE_nexofolio-admin");
    let help = std::process::Command::new(binary)
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("catalog-show"));
    for command in ["catalog-create", "catalog-run", "catalog-preview"] {
        assert!(!help.contains(command));
        let response = std::process::Command::new(binary)
            .arg(command)
            .output()
            .unwrap();
        assert_eq!(response.status.code(), Some(2));
        assert!(
            String::from_utf8(response.stderr)
                .unwrap()
                .contains("unrecognized subcommand")
        );
    }
}

// Explicit synthetic historical database fixtures: no generator, lease or task executor.
async fn historical_snapshot(
    sql: &sqlx::PgPool,
    store: &PostgresCatalogPreviews,
    project: ProjectId,
) -> PreviewTask {
    let interfaces:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('interface_id',d.id,'method',d.method,'path',d.path,'environments',jsonb_agg(jsonb_build_object('environment_id',e.id,'environment_name',e.name,'revision_id',r.id,'definition',r.definition) ORDER BY e.id)) FROM interface_documents d JOIN interface_environment_current c ON c.interface_id=d.id JOIN interface_observed_revisions r ON r.id=c.current_revision_id JOIN environments e ON e.id=c.environment_id WHERE d.project_id=$1 GROUP BY d.id ORDER BY d.id")
        .bind(uuid(project)).fetch_all(sql).await.unwrap();
    let task = JobId::new();
    sqlx::query("INSERT INTO catalog_preview_tasks(id,project_id,candidate_id,contract_version,snapshot,snapshot_sha256) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(uuid(task)).bind(uuid(project)).bind(Uuid::new_v4()).bind(CATALOG_PREVIEW_VERSION)
        .bind(json!({"project_id":project,"project_name":"Historical fixture","interfaces":interfaces})).bind("0".repeat(64)).execute(sql).await.unwrap();
    store.read(task).await.unwrap()
}
async fn save_historical_candidate(
    sql: &sqlx::PgPool,
    task: &PreviewTask,
    candidate: &DirectoryCandidate,
    review: &PreviewReview,
) {
    sqlx::query(
        "UPDATE catalog_preview_tasks SET status='ready',candidate=$2,review=$3 WHERE id=$1",
    )
    .bind(uuid(task.task_id))
    .bind(serde_json::to_value(candidate).unwrap())
    .bind(serde_json::to_value(review).unwrap())
    .execute(sql)
    .await
    .unwrap();
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
