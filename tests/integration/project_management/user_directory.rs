use super::support::*;
use nexofolio_access_contracts::UserDirectory;
use nexofolio_common::Secret;
use serde_json::json;

const PATH: &str = "/v1/user-management/users";

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn registered_users_are_paged_instance_scoped_and_use_saved_profiles() {
    let h = Harness::new().await;
    let admin = h.user("A80646").await;
    let member = h.user("MixedCase").await;
    let disabled = h.user("disabled-member").await;
    for (user, at) in [
        (&admin, "2026-10-01T01:00:00Z"),
        (&member, "2026-10-02T02:00:00Z"),
        (&disabled, "2026-10-03T03:00:00Z"),
    ] {
        sqlx::query("UPDATE access.users SET created_at=$2::text::timestamptz,last_login_at=$2::text::timestamptz WHERE id=$1")
            .bind(uuid(user.principal.user_id)).bind(at).execute(&h.sql).await.unwrap();
    }
    sqlx::query("UPDATE access.users SET enabled=false WHERE id=$1")
        .bind(uuid(disabled.principal.user_id))
        .execute(&h.sql)
        .await
        .unwrap();
    let avatar = "data:image/webp;base64,UklGRg==";
    sqlx::query("INSERT INTO access.profile_preferences(user_id,display_name,avatar_url,bio) VALUES($1,'王%_小明',$2,'private biography not part of the directory')")
        .bind(uuid(member.principal.user_id)).bind(avatar).execute(&h.sql).await.unwrap();
    let other = Harness::new().await;
    other.user("other-instance-hidden").await;
    let (status, first) = h
        .call("GET", &format!("{PATH}?limit=1"), Some(&admin.token), None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(first["total"], 3);
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    assert_eq!(first["items"][0]["id"], json!(disabled.principal.user_id));
    assert_eq!(first["items"][0]["enabled"], false);
    let (_, second) = h
        .call(
            "GET",
            &format!("{PATH}?page=2&limit=1"),
            Some(&admin.token),
            None,
        )
        .await;
    let row = &second["items"][0];
    assert_eq!(row["account"], "MixedCase");
    assert_eq!(row["display_name"], "王%_小明");
    assert_eq!(row["avatar_url"], avatar);
    assert_eq!(row["created_at"], "2026-10-02T02:00:00+00:00");
    assert_eq!(row["last_login_at"], "2026-10-02T02:00:00+00:00");
    let mut keys: Vec<_> = row
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "account",
            "avatar_url",
            "created_at",
            "display_name",
            "enabled",
            "id",
            "last_login_at"
        ]
    );
    for query in ["q=mixedcase", "q=%25_"] {
        let (_, found) = h
            .call("GET", &format!("{PATH}?{query}"), Some(&admin.token), None)
            .await;
        assert_eq!(found["total"], 1);
        assert_eq!(found["items"][0]["id"], json!(member.principal.user_id));
    }
    let (_, empty) = h
        .call(
            "GET",
            &format!("{PATH}?q=does-not-exist"),
            Some(&admin.token),
            None,
        )
        .await;
    assert_eq!(empty["total"], 0);
    assert_eq!(empty["items"], json!([]));
    // Listing must not advance a user's login timestamp.
    let at: String = sqlx::query_scalar("SELECT to_char(last_login_at AT TIME ZONE 'UTC','YYYY-MM-DD HH24:MI:SS') FROM access.users WHERE id=$1")
        .bind(uuid(member.principal.user_id)).fetch_one(&h.sql).await.unwrap();
    assert_eq!(at, "2026-10-02 02:00:00");
}

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn user_directory_is_hidden_revalidates_identity_and_exposes_no_mutations() {
    let h = Harness::new().await;
    let admin = h.user("a80646").await;
    let member = h.user("member").await;
    let near = h.user("XA80646").await;
    let invalid = Secret::new("expired-fixture");
    for token in [None, Some(&member.token), Some(&near.token), Some(&invalid)] {
        assert_eq!(
            h.call("GET", &format!("{PATH}?page=invalid"), token, None)
                .await
                .0,
            404
        );
    }
    for query in [
        "page=0",
        "page=100001",
        "limit=0",
        "limit=101",
        "unexpected=true",
        "q=%00",
        &format!("q={}", "a".repeat(101)),
    ] {
        let status = h
            .call("GET", &format!("{PATH}?{query}"), Some(&admin.token), None)
            .await
            .0;
        assert!([400, 422].contains(&status), "{query}: {status}");
    }
    for method in ["POST", "PATCH", "DELETE"] {
        assert_eq!(h.call(method, PATH, Some(&admin.token), None).await.0, 405);
    }
    assert_eq!(
        h.call("GET", PATH, Some(&admin.token), None).await.1["total"],
        3
    );
    sqlx::query("UPDATE access.internal_sessions SET issued_at=now()-interval '2 days',expires_at=now()-interval '1 day' WHERE user_id=$1")
        .bind(uuid(admin.principal.user_id)).execute(&h.sql).await.unwrap();
    assert_eq!(h.call("GET", PATH, Some(&admin.token), None).await.0, 404);
    sqlx::query("UPDATE access.internal_sessions SET issued_at=now(),expires_at=now()+interval '1 day' WHERE user_id=$1")
        .bind(uuid(admin.principal.user_id)).execute(&h.sql).await.unwrap();
    sqlx::query("UPDATE access.users SET account='A80646' WHERE id=$1")
        .bind(uuid(admin.principal.user_id))
        .execute(&h.sql)
        .await
        .unwrap();
    assert_eq!(h.call("GET", PATH, Some(&admin.token), None).await.0, 200);
    sqlx::query("UPDATE access.users SET enabled=false WHERE id=$1")
        .bind(uuid(admin.principal.user_id))
        .execute(&h.sql)
        .await
        .unwrap();
    assert_eq!(h.call("GET", PATH, Some(&admin.token), None).await.0, 404);
    assert!(
        h.store
            .list_registered_users(&admin.principal, 1, 20, "")
            .await
            .is_err()
    );
}
