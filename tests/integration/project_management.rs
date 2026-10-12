#[path = "project_management/audit.rs"]
mod audit;
// Isolated, synthetic projects only. No real ZenTao login, model call, or production mutation.
#[path = "project_management/edges.rs"]
mod edges;
#[path = "project_management/edit.rs"]
mod edit;
#[path = "project_management/list.rs"]
mod list;
#[path = "project_management/reset.rs"]
mod reset;
#[path = "project_management/support.rs"]
mod support;
#[path = "project_management/user_directory.rs"]
mod user_directory;
use nexofolio_access_contracts::{
    Environments, ExternalProject, LoginStore, McpProjectAccess, McpTokenVerifier, McpTokens,
    ProjectAccess, ProjectSnapshot,
};
use serde_json::json;
use support::*;

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn creation_is_hidden_inherits_live_grants_and_is_discovered_by_existing_mcp_token() {
    let h = Harness::new().await;
    let admin = h.user("a80646").await;
    let member = h.user("member").await;
    let outsider = h.user("outsider").await;
    let source = h.project("云售后").await;
    h.grant(&member, source).await;
    let (_, token) = h
        .databases
        .access
        .issue_mcp_token(member.principal.user_id, 1)
        .await
        .unwrap();
    for actor in [None, Some(&member.token), Some(&outsider.token)] {
        for (method, path, body) in [
            (
                "GET",
                "/v1/project-management/permission-sources".into(),
                None,
            ),
            (
                "POST",
                "/v1/project-management/projects".into(),
                Some(json!({"name":"fake","permission_source_project_id":source})),
            ),
            (
                "GET",
                format!("/v1/project-management/projects/{source}/content"),
                None,
            ),
            (
                "POST",
                format!("/v1/project-management/projects/{source}/clear"),
                Some(json!({"confirmation":"云售后"})),
            ),
        ] {
            assert_eq!(h.call(method, &path, actor, body).await.0, 404);
        }
    }
    assert_eq!(
        h.call("GET", "/v1/auth/me", Some(&admin.token), None)
            .await
            .1["capabilities"]["manage_projects"],
        true
    );
    assert!(
        h.call("GET", "/v1/auth/me", Some(&member.token), None)
            .await
            .1["capabilities"]
            .get("manage_projects")
            .is_none()
    );
    let body = json!({"name":"巴西售后","permission_source_project_id":source});
    let (status, created) = h
        .call(
            "POST",
            "/v1/project-management/projects",
            Some(&admin.token),
            Some(body.clone()),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    let project = created["project_id"].as_str().unwrap().parse().unwrap();
    assert!(
        h.store
            .require_project(&member.principal, project)
            .await
            .is_ok()
    );
    assert!(
        h.store
            .require_project(&outsider.principal, project)
            .await
            .is_err()
    );
    assert_eq!(h.endpoint_count(project).await, 0);
    assert_eq!(
        h.call(
            "POST",
            "/v1/project-management/projects",
            Some(&admin.token),
            Some(body)
        )
        .await
        .0,
        409
    );
    assert_eq!(
        h.call(
            "POST",
            "/v1/project-management/projects",
            Some(&admin.token),
            Some(json!({"name":"nested","permission_source_project_id":project}))
        )
        .await
        .0,
        404
    );
    let sources = h
        .call(
            "GET",
            "/v1/project-management/permission-sources",
            Some(&admin.token),
            None,
        )
        .await
        .1;
    assert_eq!(sources["total"], 1);
    let environment = h
        .store
        .create_environment(&member.principal, project, "测试环境")
        .await
        .unwrap();
    let principal = h.databases.access.verify(&token).await.unwrap();
    assert_eq!(
        h.databases
            .access
            .list_mcp_projects(&principal, 0, 50)
            .await
            .unwrap()
            .total,
        2
    );
    assert_eq!(
        h.databases
            .access
            .list_mcp_environments(&principal, project, 0, 50)
            .await
            .unwrap()
            .items[0]
            .environment_id,
        environment.id
    );
    let mcp = h.mcp(&token, "list_projects", json!({})).await;
    assert!(
        mcp["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["project_id"] == json!(project))
    );
    let roots = h
        .mcp(
            &token,
            "browse_directory",
            json!({"project_id":project,"environment_id":environment.id,"section":"directories"}),
        )
        .await;
    assert_eq!(roots["total"], 0);
    let members: i64 =
        sqlx::query_scalar("SELECT count(*) FROM access.user_project_access WHERE project_id=$1")
            .bind(uuid(project))
            .fetch_one(&h.sql)
            .await
            .unwrap();
    assert_eq!(members, 0);
    // Sync still updates the real project only; the local project and binding survive.
    h.store
        .apply_snapshot(
            &member.principal,
            10_000,
            ProjectSnapshot {
                projects: vec![ExternalProject {
                    instance: h.instance.clone(),
                    external_id: source.to_string(),
                    name: "云售后".into(),
                    state: "doing".into(),
                }],
                visible_project_ids: vec![],
            },
        )
        .await
        .unwrap();
    assert!(
        h.store
            .require_project(&member.principal, project)
            .await
            .is_err()
    );
    assert_eq!(
        h.databases
            .access
            .mcp_accessible_projects(&member.principal)
            .await
            .unwrap(),
        Some(0)
    );
    assert!(
        h.databases
            .access
            .require_mcp_project(&principal, project)
            .await
            .is_err()
    );
    let denied = h
        .mcp(&token, "browse_directory", json!({"project_id":project}))
        .await;
    assert!(denied.get("error").is_some());
    assert_eq!(
        h.call(
            "GET",
            "/v1/project-management/permission-sources",
            Some(&admin.token),
            None
        )
        .await
        .1["total"],
        1
    );
    let (status, me) = h.call("GET", "/v1/auth/me", Some(&admin.token), None).await;
    assert_eq!(status, 200);
    assert_eq!(me["capabilities"]["manage_projects"], true);
}
