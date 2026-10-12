use super::support::*;
use nexofolio_access_contracts::{Environments, ProjectAccess};
use nexofolio_common::ProjectId;
use serde_json::{Value, json};

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn multiple_sources_edit_preserves_content_and_updates_web_and_existing_mcp_scope() {
    let h = Harness::new().await;
    let admin = h.user("A80646").await;
    let first = h.user("first").await;
    let second = h.user("second").await;
    let both = h.user("both").await;
    let outsider = h.user("outsider").await;
    let a = h.project("云售后").await;
    let b = h.project("售后共用").await;
    for (user, project) in [(&first, a), (&second, b), (&both, a), (&both, b)] {
        h.grant(user, project).await;
    }
    let (_, token) = h
        .databases
        .access
        .issue_mcp_token(first.principal.user_id, 1)
        .await
        .unwrap();
    let (_, second_token) = h
        .databases
        .access
        .issue_mcp_token(second.principal.user_id, 1)
        .await
        .unwrap();
    let (_, both_token) = h
        .databases
        .access
        .issue_mcp_token(both.principal.user_id, 1)
        .await
        .unwrap();
    let (status, created) = h
        .call(
            "POST",
            "/v1/project-management/projects",
            Some(&admin.token),
            Some(json!({"name":"巴西售后","permission_source_project_ids":[a,b]})),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    assert_eq!(created["permission_sources"].as_array().unwrap().len(), 2);
    let p: ProjectId = created["project_id"].as_str().unwrap().parse().unwrap();
    let path = format!("/v1/project-management/projects/{p}");
    for user in [&first, &second, &both] {
        assert!(h.store.require_project(&user.principal, p).await.is_ok());
    }
    assert!(
        h.store
            .require_project(&outsider.principal, p)
            .await
            .is_err()
    );
    let projects = h.mcp(&both_token, "list_projects", json!({})).await;
    assert_eq!(projects["total"], 3);
    assert_eq!(
        projects["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["project_id"] == json!(p))
            .count(),
        1
    );
    let environment = h
        .store
        .create_environment(&first.principal, p, "测试环境")
        .await
        .unwrap();
    h.populate(p, environment.id).await;
    let before = h
        .call("GET", &format!("{path}/content"), Some(&admin.token), None)
        .await
        .1;
    let edit = json!({"name":"巴西售后业务","permission_source_project_ids":[b],"revision":1});
    let (status, updated) = h
        .call("PATCH", &path, Some(&admin.token), Some(edit.clone()))
        .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["revision"], 2);
    assert_eq!(updated["permission_sources"][0]["project_id"], json!(b));
    assert_eq!(
        h.call("GET", &path, Some(&admin.token), None).await.1,
        updated
    );
    assert_eq!(
        h.call("GET", &format!("{path}/content"), Some(&admin.token), None)
            .await
            .1,
        before
    );
    assert_eq!(h.endpoint_count(p).await, 2);
    assert_eq!(
        h.store
            .list_environments(&second.principal, p, 1, 50)
            .await
            .unwrap()
            .items[0]
            .id,
        environment.id
    );
    assert!(h.store.require_project(&first.principal, p).await.is_err());
    assert!(h.store.require_project(&second.principal, p).await.is_ok());
    let first_projects = h.mcp(&token, "list_projects", json!({})).await;
    assert!(
        !first_projects["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["project_id"] == json!(p))
    );
    assert!(
        h.mcp(&token, "browse_directory", json!({"project_id":p}))
            .await
            .get("error")
            .is_some()
    );
    let second_projects = h.mcp(&second_token, "list_projects", json!({})).await;
    assert!(
        second_projects["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["project_id"] == json!(p) && item["name"] == "巴西售后业务")
    );
    let (status, card) = h
        .call(
            "GET",
            &format!("/v1/projects/{p}"),
            Some(&admin.token),
            None,
        )
        .await;
    assert_eq!(status, 200, "{card}");
    assert_eq!(card["origin"], "local");
    assert_eq!(
        h.call("PATCH", &path, Some(&admin.token), Some(edit))
            .await
            .0,
        409
    );
    assert_eq!(
        h.call("GET", &path, Some(&admin.token), None).await.1,
        updated
    );
    let raw_grants: i64 =
        sqlx::query_scalar("SELECT count(*) FROM access.user_project_access WHERE project_id=$1")
            .bind(uuid(p))
            .fetch_one(&h.sql)
            .await
            .unwrap();
    assert_eq!(raw_grants, 0);
}

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn editing_is_hidden_validates_bindings_and_cannot_race_clear_or_overwrite_synced_projects() {
    let h = Harness::new().await;
    let admin = h.user("a80646").await;
    let member = h.user("member").await;
    let a = h.project("云售后").await;
    let local = h
        .store
        .create_local_project(&admin.principal, "本地项目", &[a])
        .await
        .unwrap();
    let p = local.project_id;
    let path = format!("/v1/project-management/projects/{p}");
    let edit = |name: &str, ids: Value| json!({"name":name,"permission_source_project_ids":ids,"revision":1});
    for token in [None, Some(&member.token)] {
        for method in ["GET", "PATCH"] {
            assert_eq!(
                h.call(method, &path, token, Some(edit("伪造", json!([a]))))
                    .await
                    .0,
                404
            );
        }
    }
    assert_eq!(
        h.call(
            "GET",
            &format!("/v1/project-management/projects/{a}"),
            Some(&admin.token),
            None
        )
        .await
        .0,
        404
    );
    assert_eq!(
        h.call(
            "PATCH",
            &format!("/v1/project-management/projects/{a}"),
            Some(&admin.token),
            Some(edit("不改禅道", json!([a])))
        )
        .await
        .0,
        404
    );
    for (ids, expected) in [
        (json!([]), 400),
        (json!([a, a]), 400),
        (json!([p]), 404),
        (json!([ProjectId::new()]), 404),
    ] {
        assert_eq!(
            h.call(
                "PATCH",
                &path,
                Some(&admin.token),
                Some(edit("不应保存", ids))
            )
            .await
            .0,
            expected
        );
    }
    // A real ZenTao source in another instance must also fail atomically.
    let foreign = ProjectId::new();
    sqlx::query("INSERT INTO access.projects(id,instance,external_id,name,status,last_sync_generation) VALUES($1,'foreign-instance','foreign','其他实例','doing',1)").bind(uuid(foreign)).execute(&h.sql).await.unwrap();
    assert_eq!(
        h.call(
            "PATCH",
            &path,
            Some(&admin.token),
            Some(edit("不应保存", json!([a, foreign])))
        )
        .await
        .0,
        404
    );
    let other = h
        .store
        .create_local_project(&admin.principal, "占用名称", &[a])
        .await
        .unwrap();
    assert_eq!(
        h.call(
            "PATCH",
            &path,
            Some(&admin.token),
            Some(edit(&other.name, json!([a])))
        )
        .await
        .0,
        409
    );
    let current = h.call("GET", &path, Some(&admin.token), None).await.1;
    assert_eq!(current["name"], "本地项目");
    assert_eq!(current["revision"], 1);
    let lease = h
        .store
        .begin_project_reset(&admin.principal, p, None)
        .await
        .unwrap();
    assert_eq!(
        h.call(
            "PATCH",
            &path,
            Some(&admin.token),
            Some(edit("编辑项目", json!([a])))
        )
        .await
        .0,
        409
    );
    drop(lease);
    assert_eq!(
        h.call(
            "PATCH",
            &path,
            Some(&admin.token),
            Some(edit("编辑项目", json!([a])))
        )
        .await
        .0,
        200
    );
}
