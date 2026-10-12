//! MCP discovery is tested through the real transport and PostgreSQL authorization.
use super::{request, uuid};
use axum::{Router, http::StatusCode};
use nexofolio_access_adapter::{Postgres, PostgresAccess};
use nexofolio_access_contracts::{McpProjectAccess, McpTokenVerifier};
use nexofolio_backend::{http, mcp::Navigation, wiring::Config};
use nexofolio_common::{EnvironmentId, Error, ProjectId, Secret, UserId};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
use nexofolio_observe::Observe;
use nexofolio_observe_contracts::testing::InMemoryObserveStore;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

async fn call(app: &Router, token: &Secret, tool: &str, args: Value) -> Value {
    let (status, response) = request(app, "POST", "/mcp", token,
        json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":tool,"arguments":args}})).await;
    assert_eq!(status, StatusCode::OK);
    response["result"]["structuredContent"].clone()
}

pub(super) async fn verify(
    database: Arc<Postgres>,
    store: Arc<PostgresAccess>,
    pool: &sqlx::PgPool,
) {
    let user = UserId::new();
    let other = UserId::new();
    for (id, name) in [(user, "discover-alice"), (other, "discover-bob")] {
        sqlx::query("INSERT INTO access.users(id,instance,external_id,account,display_name,grants_synced) VALUES($1,'discovery-fixture',$2,$2,$2,true)")
            .bind(uuid(id)).bind(name).execute(pool).await.unwrap();
    }
    let projects = [
        ProjectId::new(),
        ProjectId::new(),
        ProjectId::new(),
        ProjectId::new(),
    ];
    for (i, project) in projects.iter().enumerate() {
        sqlx::query("INSERT INTO access.projects(id,instance,external_id,name,status,last_sync_generation,last_synced_at) VALUES($1,$2,$3,$4,'doing',1,now())")
            .bind(uuid(project)).bind(if i == 3 { "another-instance" } else { "discovery-fixture" })
            .bind(i.to_string()).bind(format!("项目 {i}")).execute(pool).await.unwrap();
        if i != 2 {
            sqlx::query("INSERT INTO access.user_project_access(user_id,project_id) VALUES($1,$2)")
                .bind(uuid(user))
                .bind(uuid(project))
                .execute(pool)
                .await
                .unwrap();
        }
    }
    let environments = [
        EnvironmentId::new(),
        EnvironmentId::new(),
        EnvironmentId::new(),
    ];
    for (i, env) in environments.iter().enumerate() {
        sqlx::query("INSERT INTO access.environments(id,project_id,name) VALUES($1,$2,$3)")
            .bind(uuid(env))
            .bind(uuid(if i == 2 { projects[2] } else { projects[0] }))
            .bind(["测试环境", "生产环境", "不可见环境"][i])
            .execute(pool)
            .await
            .unwrap();
    }
    let (token_id, token) = database.issue_mcp_token(user, 7).await.unwrap();
    let (_, other_token) = database.issue_mcp_token(other, 7).await.unwrap();
    let principal = database.verify(&token).await.unwrap();
    let config = Config::from_lookup(|key| {
        (key == "DATABASE_URL").then(|| "postgres://unused@localhost/unused".into())
    })
    .unwrap();
    let observations = InMemoryObserveStore::default();
    let observe = Observe::new(observations.clone());
    let knowledge = Knowledge::new(InMemoryKnowledge::default(), Observe::new(observations));
    let app = http::router_with_navigation(
        &config,
        database.clone(),
        database.clone(),
        CancellationToken::new(),
        Some(http::mcp_tokens::routes(store, database.clone())),
        Some(Navigation::new(
            database.clone(),
            Arc::new(observe),
            Arc::new(knowledge),
        )),
    );
    let (status, initialized) = request(&app, "POST", "/mcp", &token,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"discovery-test","version":"1"}}})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        initialized["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("list_projects")
    );
    let (_, listed) = request(
        &app,
        "POST",
        "/mcp",
        &token,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    )
    .await;
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 11);
    let discovery_schema =
        &tools.iter().find(|t| t["name"] == "list_projects").unwrap()["inputSchema"];
    assert!(discovery_schema["properties"].get("project_id").is_none());

    // Discovery requires only the token; never a pre-known ID or another account identifier.
    let first = call(&app, &token, "list_projects", json!({"limit":1})).await;
    assert_eq!(first["total"], 2);
    assert_eq!(first["has_more"], true);
    assert_eq!(first["next_offset"], 1);
    let second = call(
        &app,
        &token,
        "list_projects",
        json!({"offset":first["next_offset"],"limit":1}),
    )
    .await;
    assert_eq!(second["has_more"], false);
    assert!(second["next_offset"].is_null());
    assert_ne!(
        first["items"][0]["project_id"],
        second["items"][0]["project_id"]
    );
    let all = call(&app, &token, "list_projects", json!({})).await;
    let ids: Vec<_> = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["project_id"].clone())
        .collect();
    assert!(ids.contains(&json!(projects[0])) && ids.contains(&json!(projects[1])));
    assert!(!ids.contains(&json!(projects[2])) && !ids.contains(&json!(projects[3])));
    assert_eq!(
        call(&app, &other_token, "list_projects", json!({})).await["total"],
        0
    );
    let beyond = call(&app, &token, "list_projects", json!({"offset":500})).await;
    assert_eq!(beyond["items"], json!([]));
    assert_eq!(beyond["has_more"], false);
    for args in [
        json!({"limit":0}),
        json!({"limit":51}),
        json!({"offset":-1}),
        json!({"user_id":other}),
        json!({"project_id":projects[0]}),
    ] {
        assert!(
            call(&app, &token, "list_projects", args)
                .await
                .get("error")
                .is_some()
        );
    }
    assert!(
        call(&app, &token, "list_project_environments", json!({}))
            .await
            .get("error")
            .is_some()
    );
    for project in [projects[2], projects[3], ProjectId::new()] {
        assert_eq!(
            call(
                &app,
                &token,
                "list_project_environments",
                json!({"project_id":project})
            )
            .await["error"],
            "PROJECT_ACCESS_DENIED"
        );
    }
    // Pass IDs returned by discovery directly into subsequent calls.
    let selected = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "项目 0")
        .unwrap();
    let env_page = call(
        &app,
        &token,
        "list_project_environments",
        json!({"project_id":selected["project_id"],"limit":1}),
    )
    .await;
    assert_eq!(env_page["total"], 2);
    assert_eq!(env_page["has_more"], true);
    let next_env = call(
        &app,
        &token,
        "list_project_environments",
        json!({"project_id":selected["project_id"],"limit":1,"offset":env_page["next_offset"]}),
    )
    .await;
    assert_eq!(next_env["has_more"], false);
    assert_ne!(
        env_page["items"][0]["environment_id"],
        next_env["items"][0]["environment_id"]
    );
    assert_eq!(env_page["items"][0]["project_id"], selected["project_id"]);
    let found = call(&app, &token, "search_interfaces", json!({"project_id":selected["project_id"],"environment_id":env_page["items"][0]["environment_id"]})).await;
    assert_eq!(found["total"], 0); // No captures in this isolated fixture; discovery still works.
    assert_eq!(
        call(
            &app,
            &token,
            "list_project_environments",
            json!({"project_id":projects[1]})
        )
        .await["total"],
        0
    );
    sqlx::query("DELETE FROM access.user_project_access WHERE user_id=$1 AND project_id=$2")
        .bind(uuid(user))
        .bind(uuid(projects[0]))
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, &token, "list_projects", json!({})).await["total"],
        1
    );
    assert_eq!(
        call(
            &app,
            &token,
            "list_project_environments",
            json!({"project_id":projects[0]})
        )
        .await["error"],
        "PROJECT_ACCESS_DENIED"
    );

    // Reused principals must not bypass revocation, expiry, owner status, or grant readiness.
    for state in ["disabled", "unsynced", "scheduled", "expired", "revoked"] {
        match state {
            "disabled" => {
                sqlx::query("UPDATE access.users SET enabled=false WHERE id=$1")
                    .bind(uuid(user))
                    .execute(pool)
                    .await
                    .unwrap();
            }
            "unsynced" => {
                sqlx::query("UPDATE access.users SET grants_synced=false WHERE id=$1")
                    .bind(uuid(user))
                    .execute(pool)
                    .await
                    .unwrap();
            }
            "scheduled" => {
                sqlx::query(
                    "UPDATE access.mcp_tokens SET starts_at=now()+interval '1 day' WHERE id=$1",
                )
                .bind(uuid(token_id))
                .execute(pool)
                .await
                .unwrap();
            }
            "expired" => {
                sqlx::query("UPDATE access.mcp_tokens SET issued_at=now()-interval '3 days',starts_at=now()-interval '2 days',expires_at=now()-interval '1 day' WHERE id=$1").bind(uuid(token_id)).execute(pool).await.unwrap();
            }
            _ => {
                database.revoke_mcp_token(token_id).await.unwrap();
            }
        }
        assert!(matches!(
            database.list_mcp_projects(&principal, 0, 20).await,
            Err(Error::Unauthenticated)
        ));
        assert!(matches!(
            database
                .list_mcp_environments(&principal, projects[1], 0, 20)
                .await,
            Err(Error::Unauthenticated)
        ));
        sqlx::query("UPDATE access.users SET enabled=true,grants_synced=true WHERE id=$1")
            .bind(uuid(user))
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE access.mcp_tokens SET starts_at=now(),expires_at=now()+interval '7 days',revoked_at=NULL WHERE id=$1").bind(uuid(token_id)).execute(pool).await.unwrap();
    }
}
