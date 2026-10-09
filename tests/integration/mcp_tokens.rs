//! Isolated PostgreSQL + real HTTP/MCP middleware; no model or running service.
#[path = "mcp_discovery.rs"]
mod discovery;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use nexofolio_access_adapter::{Postgres, PostgresAccess};
use nexofolio_access_contracts::{
    ExternalIdentity, LoginStore, McpProjectAccess, McpTokenVerifier, Sessions,
};
use nexofolio_backend::{http, wiring::Config};
use nexofolio_common::{Error, ProjectId, Secret};
use serde_json::{Value, json};
use sqlx::{ConnectOptions, postgres::PgConnectOptions};
use std::{str::FromStr, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    token: &Secret,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {}", token.expose()))
                .header("content-type", "application/json")
                .header("host", "localhost")
                .header("mcp-protocol-version", "2025-11-25")
                .header("accept", "application/json, text/event-stream")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    if path.starts_with("/v1/auth/mcp-") {
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    }
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}
fn uuid(id: impl std::fmt::Display) -> uuid::Uuid {
    id.to_string().parse().unwrap()
}

#[tokio::test]
#[ignore = "requires a dedicated isolated TEST_DATABASE_URL"]
async fn account_owned_credentials_enforce_validity_and_live_grants_without_session_reuse() {
    let url = Secret::new(std::env::var("TEST_DATABASE_URL").expect("isolated database required"));
    let options = PgConnectOptions::from_str(url.expose())
        .unwrap()
        .options([("search_path", "access")])
        .disable_statement_logging();
    let pool = sqlx::PgPool::connect_with(options).await.unwrap();
    // Refuse accidental execution against any ordinary application database.
    let db_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(db_name.starts_with("nexofolio_mcp_test"));
    // Build the previous schema and credential first, then exercise the upgrade.
    let mut previous = sqlx::migrate!("../../modules/access/adapter/migrations");
    previous
        .migrations
        .to_mut()
        .retain(|m| m.version < 202610090001);
    previous.create_schema("access");
    previous.dangerous_set_table_name("access._sqlx_migrations");
    previous.run(&pool).await.unwrap();
    use sha2::{Digest, Sha256};
    let legacy = Secret::new(format!("nfi_{}", "42".repeat(32)));
    let legacy_owner = nexofolio_common::UserId::new();
    let legacy_id = nexofolio_common::TokenId::new();
    sqlx::query("INSERT INTO access.users(id,instance,external_id,account,display_name,grants_synced) VALUES($1,'legacy-fixture','legacy','legacy','Legacy',true)")
        .bind(uuid(legacy_owner)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO access.mcp_tokens(id,user_id,token_hash,expires_at) VALUES($1,$2,$3,now()+interval '7 days')")
        .bind(uuid(legacy_id)).bind(uuid(legacy_owner)).bind(Sha256::digest(legacy.expose().as_bytes()).to_vec()).execute(&pool).await.unwrap();
    let database = Arc::new(Postgres::new(&url, 4, Duration::from_secs(5)).unwrap());
    database.migrate().await.unwrap();
    assert!(database.verify(&legacy).await.is_ok());
    let preserved: bool = sqlx::query_scalar("SELECT starts_at=issued_at AND expires_at>now() AND revoked_at IS NULL FROM access.mcp_tokens WHERE id=$1")
        .bind(uuid(legacy_id)).fetch_one(&pool).await.unwrap();
    assert!(preserved);
    let store = Arc::new(
        PostgresAccess::new(database.as_ref().clone(), &Secret::new("ab".repeat(32))).unwrap(),
    );
    let identity = ExternalIdentity {
        instance: "mcp-management-fixture".into(),
        external_id: "alice".into(),
        account: "alice".into(),
        display_name: "Alice".into(),
    };
    let alice = store.normal_login(&identity).await.unwrap();
    let bob = store
        .normal_login(&ExternalIdentity {
            external_id: "bob".into(),
            account: "bob".into(),
            ..identity
        })
        .await
        .unwrap();
    sqlx::query("UPDATE access.users SET grants_synced=true")
        .execute(&pool)
        .await
        .unwrap();
    let config = Config::from_lookup(|key| match key {
        "DATABASE_URL" => Some("postgres://unused@127.0.0.1/unused".into()),
        _ => None,
    })
    .unwrap();
    let app = http::router_with_access(
        &config,
        database.clone(),
        database.clone(),
        CancellationToken::new(),
        Some(http::mcp_tokens::routes(store.clone(), database.clone())),
    );
    let path = "/v1/auth/mcp-tokens";
    let connection_path = "/v1/auth/mcp-connection";
    let (status, connection) =
        request(&app, "GET", connection_path, &alice.token, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(connection["transport"], "Streamable HTTP");
    assert_eq!(connection["accessible_projects"], 0);
    use rmcp::ServerHandler;
    let expected_versions = nexofolio_backend::mcp::KnowledgeMcp::default()
        .supported_protocol_versions()
        .iter()
        .rev()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert_eq!(connection["protocol_versions"], json!(expected_versions));
    let (status, value) = request(
        &app,
        "POST",
        path,
        &alice.token,
        json!({"name":"  Alice 客户端  ","starts_at":null,"expires_at":null}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let token = Secret::new(value["token"].as_str().unwrap());
    assert!(token.expose().starts_with("nfm_"));
    let id = value["credential"]["id"].as_str().unwrap();
    assert_eq!(value["credential"]["name"], "Alice 客户端");
    assert_eq!(value["credential"]["status"], "active");
    assert!(value["credential"]["expires_at"].is_null());
    assert!(matches!(
        store.verify_session(&token).await,
        Err(Error::Unauthenticated)
    ));
    assert!(matches!(
        database.verify(&alice.token).await,
        Err(Error::Unauthenticated)
    ));
    assert_eq!(
        request(&app, "GET", path, &token, Value::Null).await.0,
        StatusCode::UNAUTHORIZED
    );
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"isolated-test","version":"1"}}});
    assert_eq!(
        request(&app, "POST", "/mcp", &alice.token, initialize.clone())
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, "POST", "/mcp", &token, initialize.clone())
            .await
            .0,
        StatusCode::OK
    );

    let (_, list) = request(&app, "GET", path, &alice.token, Value::Null).await;
    assert_eq!(list["total"], 1);
    assert!(!list.to_string().contains(token.expose()));
    assert!(list["items"][0].get("token_hash").is_none());
    let (_, list) = request(&app, "GET", path, &bob.token, Value::Null).await;
    assert_eq!(list["total"], 0);
    let revoke_path = format!("{path}/{id}");
    assert_eq!(
        request(&app, "DELETE", &revoke_path, &bob.token, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert!(database.verify(&token).await.is_ok());
    for input in [
        json!({"name":"x","user_id":bob.user.id}),
        json!({"name":"x","project_id":ProjectId::new()}),
    ] {
        assert_eq!(
            request(&app, "POST", path, &alice.token, input).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    for input in [
        json!({"name":"  "}),
        json!({"name":"x","expires_at":"invalid"}),
        json!({"name":"x","starts_at":"2099-01-02T00:00:00Z","expires_at":"2099-01-01T00:00:00Z"}),
        json!({"name":"x","expires_at":"2000-01-01T00:00:00Z"}),
    ] {
        assert_eq!(
            request(&app, "POST", path, &alice.token, input).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    let (status, future) = request(&app, "POST", path, &alice.token, json!({"name":"scheduled","starts_at":"2099-01-01T08:00:00+08:00","expires_at":"2099-01-02T00:00:00Z"})).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(future["credential"]["status"], "scheduled");
    let future_token = Secret::new(future["token"].as_str().unwrap());
    assert!(matches!(
        database.verify(&future_token).await,
        Err(Error::Unauthenticated)
    ));
    let (_, first) = request(
        &app,
        "GET",
        &format!("{path}?page=1&limit=1"),
        &alice.token,
        Value::Null,
    )
    .await;
    let (_, second) = request(
        &app,
        "GET",
        &format!("{path}?page=2&limit=1"),
        &alice.token,
        Value::Null,
    )
    .await;
    assert_eq!(first["total"], 2);
    assert_ne!(first["items"][0]["id"], second["items"][0]["id"]);
    assert_eq!(
        request(
            &app,
            "GET",
            &format!("{path}?page=0"),
            &alice.token,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );

    let principal = database.verify(&token).await.unwrap();
    let projects = [ProjectId::new(), ProjectId::new()];
    for (index, project) in projects.iter().enumerate() {
        sqlx::query("INSERT INTO access.projects(id,instance,external_id,name,status,last_sync_generation,last_synced_at) VALUES($1,'mcp-management-fixture',$2,'Fixture','doing',1,now())")
            .bind(uuid(project)).bind(index.to_string()).execute(&pool).await.unwrap();
        assert!(
            database
                .require_mcp_project(&principal, *project)
                .await
                .is_err()
        );
        sqlx::query("INSERT INTO access.user_project_access(user_id,project_id) VALUES($1,$2)")
            .bind(uuid(alice.user.id))
            .bind(uuid(project))
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            database
                .require_mcp_project(&principal, *project)
                .await
                .is_ok()
        );
    }
    let (_, connection) = request(&app, "GET", connection_path, &alice.token, Value::Null).await;
    assert_eq!(connection["accessible_projects"], 2);
    let (_, other_connection) =
        request(&app, "GET", connection_path, &bob.token, Value::Null).await;
    assert_eq!(other_connection["accessible_projects"], 0);
    sqlx::query("UPDATE access.users SET grants_synced=false WHERE id=$1")
        .bind(uuid(alice.user.id))
        .execute(&pool)
        .await
        .unwrap();
    let (_, unknown_connection) =
        request(&app, "GET", connection_path, &alice.token, Value::Null).await;
    assert!(unknown_connection["accessible_projects"].is_null());
    sqlx::query("UPDATE access.users SET grants_synced=true WHERE id=$1")
        .bind(uuid(alice.user.id))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM access.user_project_access WHERE project_id=$1")
        .bind(uuid(projects[0]))
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        database
            .require_mcp_project(&principal, projects[0])
            .await
            .is_err()
    );
    let (_, connection) = request(&app, "GET", connection_path, &alice.token, Value::Null).await;
    assert_eq!(connection["accessible_projects"], 1);
    assert!(
        database
            .require_mcp_project(&principal, projects[1])
            .await
            .is_ok()
    );
    sqlx::query("UPDATE access.users SET enabled=false WHERE id=$1")
        .bind(uuid(alice.user.id))
        .execute(&pool)
        .await
        .unwrap();
    assert!(database.verify(&token).await.is_err());
    assert!(
        database
            .require_mcp_project(&principal, projects[1])
            .await
            .is_err()
    );
    sqlx::query("UPDATE access.users SET enabled=true WHERE id=$1")
        .bind(uuid(alice.user.id))
        .execute(&pool)
        .await
        .unwrap();

    // A previously issued principal cannot bypass start/expiry/revocation checks.
    sqlx::query("UPDATE access.mcp_tokens SET starts_at=now()+interval '1 day' WHERE id=$1")
        .bind(uuid(id))
        .execute(&pool)
        .await
        .unwrap();
    assert!(database.verify(&token).await.is_err());
    assert!(
        database
            .require_mcp_project(&principal, projects[1])
            .await
            .is_err()
    );
    sqlx::query("UPDATE access.mcp_tokens SET issued_at=now()-interval '3 days',starts_at=now()-interval '2 days',expires_at=now()-interval '1 day' WHERE id=$1").bind(uuid(id)).execute(&pool).await.unwrap();
    assert!(database.verify(&token).await.is_err());
    assert!(
        database
            .require_mcp_project(&principal, projects[1])
            .await
            .is_err()
    );
    let (_, expired) = request(&app, "GET", path, &alice.token, Value::Null).await;
    assert!(
        expired["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == id && t["status"] == "expired")
    );
    sqlx::query("UPDATE access.mcp_tokens SET expires_at=NULL WHERE id=$1")
        .bind(uuid(id))
        .execute(&pool)
        .await
        .unwrap();
    assert!(database.verify(&token).await.is_ok());
    assert_eq!(
        request(&app, "DELETE", &revoke_path, &alice.token, Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, "DELETE", &revoke_path, &alice.token, Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert!(database.verify(&token).await.is_err());
    assert!(
        database
            .require_mcp_project(&principal, projects[1])
            .await
            .is_err()
    );
    assert_eq!(
        request(&app, "POST", "/mcp", &token, initialize).await.0,
        StatusCode::UNAUTHORIZED
    );

    // Old MCP credentials share the old nfi_ prefix but never the session table.
    assert!(database.verify(&legacy).await.is_ok());
    assert!(store.verify_session(&legacy).await.is_err());
    discovery::verify(database.clone(), store.clone(), &pool).await;
    drop(app);
    drop(store);
    database.close().await;
    pool.close().await;
}
