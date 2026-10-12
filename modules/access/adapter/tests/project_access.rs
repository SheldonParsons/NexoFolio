//! Real PostgreSQL authorization: no synthetic membership for the account exception.
#[path = "project_access/mcp.rs"]
mod mcp;
#[path = "project_access/support.rs"]
mod support;

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn public_document_metadata_does_not_grant_member_or_mcp_access() {
    use nexofolio_access_contracts::ProjectDocuments;
    let h = Harness::new().await;
    let project = h.project(&h.instance, "doing").await;
    let (user, _) = h.user("public-outsider", true).await;
    let foreign = h.project("public-other-instance", "doing").await;
    let info = h
        .database
        .document_project(&h.instance, project)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(info.project_id, project);
    assert!(
        h.database
            .document_project(&h.instance, foreign)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        h.database
            .document_project(&h.instance, ProjectId::new())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(h.members(user.user_id).await, 0);
    assert!(matches!(
        h.store.require_project(&user, project).await,
        Err(Error::Forbidden)
    ));
    let (_, token) = h.database.issue_mcp_token(user.user_id, 1).await.unwrap();
    let principal = h.database.verify(&token).await.unwrap();
    assert!(
        h.database
            .require_mcp_project(&principal, project)
            .await
            .is_err()
    );
    assert_eq!(
        h.database
            .list_mcp_projects(&principal, 0, 20)
            .await
            .unwrap()
            .total,
        0
    );
}

use nexofolio_access_contracts::{
    AccessState, Environments, McpProjectAccess, McpTokenVerifier, McpTokens, ProjectAccess,
    Sessions,
};
use nexofolio_common::{Error, ProjectId, Secret};
use support::{Harness, uuid};

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn account_exception_covers_web_environment_writes_and_future_projects_without_membership() {
    let h = Harness::new().await;
    let (user, token) = h.user("A80646", false).await;
    let project = h.project(&h.instance, "closed").await;
    let foreign = h.project("foreign-instance", "doing").await;

    for account in ["A80646", "a80646"] {
        h.rename(user.user_id, account).await;
        let listed = h.store.list_projects(&user, 1, 100).await.unwrap();
        assert_eq!(listed.total, 1);
        assert!(listed.items[0].can_access);
        assert!(matches!(listed.items[0].access_state, AccessState::Allowed));
        assert!(h.store.require_project(&user, project).await.is_ok());
    }
    let next = h.project(&h.instance, "wait").await;
    assert!(h.store.require_project(&user, next).await.is_ok());
    assert_eq!(h.store.list_projects(&user, 1, 100).await.unwrap().total, 2);
    assert!(matches!(
        h.store.require_project(&user, foreign).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        h.store.require_project(&user, ProjectId::new()).await,
        Err(Error::NotFound)
    ));

    let env = h
        .store
        .create_environment(&user, project, "测试环境")
        .await
        .unwrap();
    h.store
        .rename_environment(&user, project, env.id, "灰度环境")
        .await
        .unwrap();
    let envs = h
        .store
        .list_environments(&user, project, 1, 20)
        .await
        .unwrap();
    assert_eq!(envs.items[0].name, "灰度环境");
    assert_eq!(h.members(user.user_id).await, 0);

    h.disable(user.user_id).await;
    assert!(matches!(
        h.store.verify_session(&token).await,
        Err(Error::Unauthenticated)
    ));
    assert!(h.store.require_project(&user, project).await.is_err());
    assert!(
        h.store
            .create_environment(&user, project, "拒绝")
            .await
            .is_err()
    );
    assert!(
        h.store
            .list_projects(&user, 1, 100)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        h.store
            .verify_session(&Secret::new("A80646"))
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn ordinary_users_still_require_synced_grants_and_cannot_gain_access_by_near_match() {
    let h = Harness::new().await;
    let project = h.project(&h.instance, "doing").await;
    let foreign = h.project("other-ordinary-instance", "doing").await;
    let (user, _) = h.user("ordinary", false).await;
    h.grant(user.user_id, project).await;
    h.grant(user.user_id, foreign).await;
    assert!(matches!(
        h.store.require_project(&user, project).await,
        Err(Error::Unavailable {
            component: "project_access"
        })
    ));
    assert_eq!(
        h.database.mcp_accessible_projects(&user).await.unwrap(),
        None
    );
    assert!(h.database.issue_mcp_token(user.user_id, 1).await.is_err());
    sqlx::query("UPDATE users SET grants_synced=true WHERE id=$1")
        .bind(uuid(user.user_id))
        .execute(&h.pool)
        .await
        .unwrap();
    assert!(h.store.require_project(&user, project).await.is_ok());
    assert!(matches!(
        h.store.require_project(&user, foreign).await,
        Err(Error::NotFound)
    ));
    let (_, token) = h.database.issue_mcp_token(user.user_id, 1).await.unwrap();
    let principal = h.database.verify(&token).await.unwrap();
    assert_eq!(
        h.database
            .list_mcp_projects(&principal, 0, 20)
            .await
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        h.database.mcp_accessible_projects(&user).await.unwrap(),
        Some(1)
    );
    assert!(
        h.database
            .require_mcp_project(&principal, project)
            .await
            .is_ok()
    );
    sqlx::query("DELETE FROM user_project_access WHERE user_id=$1 AND project_id=$2")
        .bind(uuid(user.user_id))
        .bind(uuid(project))
        .execute(&h.pool)
        .await
        .unwrap();
    assert!(
        h.database
            .require_mcp_project(&principal, project)
            .await
            .is_err()
    );
    assert!(matches!(
        h.store.require_project(&user, project).await,
        Err(Error::Forbidden)
    ));

    for account in ["A806460", "xA80646", "A80646 ", "A8064"] {
        let (other, _) = h.user(account, true).await;
        assert!(matches!(
            h.store.require_project(&other, project).await,
            Err(Error::Forbidden)
        ));
        assert_eq!(
            h.database.mcp_accessible_projects(&other).await.unwrap(),
            Some(0)
        );
        let (_, token) = h.database.issue_mcp_token(other.user_id, 1).await.unwrap();
        let principal = h.database.verify(&token).await.unwrap();
        assert_eq!(
            h.database
                .list_mcp_projects(&principal, 0, 20)
                .await
                .unwrap()
                .total,
            0
        );
        assert!(
            h.database
                .require_mcp_project(&principal, project)
                .await
                .is_err()
        );
    }
}
