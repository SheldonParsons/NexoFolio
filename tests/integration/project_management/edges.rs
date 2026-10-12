use super::support::*;
use nexofolio_access_contracts::Environments;
use serde_json::json;

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn management_is_hidden_before_method_or_payload_validation() {
    let h = Harness::new().await;
    let member = h.user("ordinary").await;
    for token in [None, Some(&member.token)] {
        for path in [
            "/v1/project-management/projects",
            "/v1/project-management/permission-sources",
            "/v1/project-management/projects/invalid-id/clear",
            "/v1/project-management/projects/invalid-id",
        ] {
            for method in ["GET", "POST", "PATCH", "HEAD", "DELETE", "OPTIONS"] {
                assert_eq!(
                    h.call(
                        method,
                        path,
                        token,
                        Some(json!({"account":"A80646","unexpected":true}))
                    )
                    .await
                    .0,
                    404,
                    "{method} {path}"
                );
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn clear_deletes_aliases_without_reactivating_duplicate_endpoint_identities() {
    let h = Harness::new().await;
    let admin = h.user("a80646").await;
    let project = h.project("别名测试").await;
    let environment = h
        .store
        .create_environment(&admin.principal, project, "测试环境")
        .await
        .unwrap();
    let (_, ids) = h.populate(project, environment.id).await;
    sqlx::query("INSERT INTO observe.endpoints(id,project_id,method,path_template,merged_into) SELECT $1,project_id,method,path_template,id FROM observe.endpoints WHERE id=$2").bind(uuid::Uuid::new_v4()).bind(ids[0]).execute(&h.sql).await.unwrap();
    assert_eq!(h.endpoint_count(project).await, 3);
    let (status, data) = h
        .call(
            "POST",
            &format!("/v1/project-management/projects/{project}/clear"),
            Some(&admin.token),
            Some(json!({"confirmation":"别名测试"})),
        )
        .await;
    assert_eq!(status, 200, "{data}");
    assert_eq!(data["cleared"]["interfaces"], 2);
    assert_eq!(h.endpoint_count(project).await, 0);
}
