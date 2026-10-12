use super::support::{Harness, uuid};
use nexofolio_access_contracts::{
    CreateMcpToken, Environments, McpProjectAccess, McpTokenVerifier, McpTokens, ProjectAccess,
};

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn mcp_uses_live_exception_for_discovery_authorization_and_counts_without_grant_rows() {
    let h = Harness::new().await;
    let (user, _) = h.user("a80646", false).await;
    let first = h.project(&h.instance, "doing").await;
    let second = h.project(&h.instance, "closed").await;
    let foreign = h.project("mcp-foreign-instance", "doing").await;
    // Even a corrupt cross-instance membership must not punch through the boundary.
    h.grant(user.user_id, foreign).await;
    let env = h
        .store
        .create_environment(&user, second, "测试环境")
        .await
        .unwrap();
    let (_, token) = h.database.issue_mcp_token(user.user_id, 1).await.unwrap();
    let principal = h.database.verify(&token).await.unwrap();
    let page = h
        .database
        .list_mcp_projects(&principal, 0, 1)
        .await
        .unwrap();
    assert_eq!(page.total, 2);
    assert!(page.has_more);
    let last = h
        .database
        .list_mcp_projects(&principal, page.next_offset.unwrap(), 1)
        .await
        .unwrap();
    assert!(!last.has_more);
    assert_ne!(page.items[0].project_id, last.items[0].project_id);
    let envs = h
        .database
        .list_mcp_environments(&principal, second, 0, 20)
        .await
        .unwrap();
    assert_eq!(envs.items[0].environment_id, env.id);
    assert!(
        h.database
            .require_mcp_project(&principal, first)
            .await
            .is_ok()
    );
    assert!(
        h.database
            .require_mcp_project(&principal, second)
            .await
            .is_ok()
    );
    assert!(
        h.database
            .require_mcp_project(&principal, foreign)
            .await
            .is_err()
    );
    assert!(
        h.database
            .list_mcp_environments(&principal, foreign, 0, 20)
            .await
            .is_err()
    );
    assert_eq!(
        h.database.mcp_accessible_projects(&user).await.unwrap(),
        Some(2)
    );
    let future = h.project(&h.instance, "wait").await;
    assert!(
        h.database
            .require_mcp_project(&principal, future)
            .await
            .is_ok()
    );
    assert_eq!(
        h.database
            .list_mcp_projects(&principal, 0, 20)
            .await
            .unwrap()
            .total,
        3
    );
    assert_eq!(
        h.database.mcp_accessible_projects(&user).await.unwrap(),
        Some(3)
    );
    assert_eq!(
        h.members(user.user_id).await,
        1,
        "only the deliberately seeded foreign grant exists"
    );

    // Re-evaluate the stored identity on every call, including an already verified principal.
    h.rename(user.user_id, "former-admin").await;
    assert!(h.database.verify(&token).await.is_err());
    assert!(
        h.database
            .list_mcp_projects(&principal, 0, 20)
            .await
            .is_err()
    );
    assert!(
        h.database
            .require_mcp_project(&principal, first)
            .await
            .is_err()
    );
    assert!(h.store.require_project(&user, first).await.is_err());
    h.rename(user.user_id, "A80646").await;
    h.disable(user.user_id).await;
    assert!(h.database.verify(&token).await.is_err());
    assert!(
        h.database
            .require_mcp_project(&principal, first)
            .await
            .is_err()
    );
    assert!(h.database.mcp_accessible_projects(&user).await.is_err());
}

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn exception_never_bypasses_mcp_expiry_revocation_or_owner_identity() {
    let h = Harness::new().await;
    let (user, _) = h.user("A80646", true).await;
    let (other, _) = h.user("ordinary", true).await;
    let project = h.project(&h.instance, "doing").await;
    let created = h
        .database
        .create_mcp_token(
            &user,
            CreateMcpToken {
                name: "fixture MCP".into(),
                starts_at: None,
                expires_at: None,
            },
        )
        .await
        .unwrap();
    let principal = h.database.verify(&created.token).await.unwrap();
    let mut forged = principal.clone();
    forged.user_id = other.user_id;
    assert!(h.database.list_mcp_projects(&forged, 0, 20).await.is_err());
    assert!(
        h.database
            .require_mcp_project(&forged, project)
            .await
            .is_err()
    );

    sqlx::query("UPDATE mcp_tokens SET starts_at=now()+interval '1 day' WHERE id=$1")
        .bind(uuid(principal.token_id))
        .execute(&h.pool)
        .await
        .unwrap();
    assert!(h.database.verify(&created.token).await.is_err());
    assert!(
        h.database
            .require_mcp_project(&principal, project)
            .await
            .is_err()
    );
    sqlx::query("UPDATE mcp_tokens SET issued_at=now()-interval '3 days',starts_at=now()-interval '2 days',expires_at=now()-interval '1 day' WHERE id=$1")
        .bind(uuid(principal.token_id)).execute(&h.pool).await.unwrap();
    assert!(h.database.verify(&created.token).await.is_err());
    assert!(
        h.database
            .list_mcp_environments(&principal, project, 0, 20)
            .await
            .is_err()
    );
    sqlx::query("UPDATE mcp_tokens SET expires_at=NULL WHERE id=$1")
        .bind(uuid(principal.token_id))
        .execute(&h.pool)
        .await
        .unwrap();
    h.database
        .revoke_owned_mcp_token(&user, principal.token_id)
        .await
        .unwrap();
    assert!(h.database.verify(&created.token).await.is_err());
    assert!(
        h.database
            .list_mcp_projects(&principal, 0, 20)
            .await
            .is_err()
    );
    assert!(
        h.database
            .require_mcp_project(&principal, project)
            .await
            .is_err()
    );
    assert_eq!(h.members(user.user_id).await, 0);
}
