use super::support::*;
use nexofolio_common::Secret;

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn management_list_is_paged_filtered_bound_and_hidden_for_invalid_identity() {
    let h = Harness::new().await;
    let admin = h.user("A80646").await;
    let member = h.user("member").await;
    let a = h.project("Z 权限来源").await;
    let b = h.project("Y 权限来源").await;
    h.store
        .create_local_project(&admin.principal, "A 独立业务", &[a, b])
        .await
        .unwrap();
    h.store
        .create_local_project(&admin.principal, "B 100%_业务", &[b])
        .await
        .unwrap();
    let path = "/v1/project-management/projects";
    let (status, page) = h
        .call(
            "GET",
            &format!("{path}?page=1&limit=1"),
            Some(&admin.token),
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(page["total"], 4);
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["origin"], "local");
    assert_eq!(
        page["items"][0]["permission_sources"][0]["name"],
        "Y 权限来源"
    );
    assert_eq!(
        page["items"][0]["permission_sources"][1]["name"],
        "Z 权限来源"
    );
    let (_, second) = h
        .call(
            "GET",
            &format!("{path}?page=2&limit=1&origin=local"),
            Some(&admin.token),
            None,
        )
        .await;
    assert_eq!(second["total"], 2);
    assert_eq!(second["items"][0]["name"], "B 100%_业务");
    let (_, exact) = h
        .call("GET", &format!("{path}?q=%25_"), Some(&admin.token), None)
        .await;
    assert_eq!(exact["total"], 1);
    let (_, synced) = h
        .call(
            "GET",
            &format!("{path}?origin=zentao"),
            Some(&admin.token),
            None,
        )
        .await;
    assert_eq!(synced["total"], 2);
    assert_eq!(
        synced["items"][0]["permission_sources"],
        serde_json::json!([])
    );
    for query in ["page=0", "limit=101", "origin=other", "unexpected=true"] {
        assert!(
            h.call("GET", &format!("{path}?{query}"), Some(&admin.token), None)
                .await
                .0
                >= 400
        );
    }
    let other = Harness::new().await;
    other.project("其他实例不应显示").await;
    assert_eq!(
        h.call("GET", path, Some(&admin.token), None).await.1["total"],
        4
    );
    for token in [None, Some(&member.token), Some(&Secret::new("expired"))] {
        assert_eq!(
            h.call("GET", &format!("{path}?page=invalid"), token, None)
                .await
                .0,
            404
        );
    }
    sqlx::query("UPDATE access.internal_sessions SET issued_at=now()-interval '2 days',expires_at=now()-interval '1 day' WHERE user_id=$1").bind(uuid(admin.principal.user_id)).execute(&h.sql).await.unwrap();
    assert_eq!(h.call("GET", path, Some(&admin.token), None).await.0, 404);
    sqlx::query("UPDATE access.internal_sessions SET issued_at=now(),expires_at=now()+interval '1 day' WHERE user_id=$1").bind(uuid(admin.principal.user_id)).execute(&h.sql).await.unwrap();
    sqlx::query("UPDATE access.users SET enabled=false WHERE id=$1")
        .bind(uuid(admin.principal.user_id))
        .execute(&h.sql)
        .await
        .unwrap();
    assert_eq!(h.call("GET", path, Some(&admin.token), None).await.0, 404);
    assert!(
        h.store
            .management_projects(&admin.principal, 1, 20, "", None)
            .await
            .is_err()
    );
}
