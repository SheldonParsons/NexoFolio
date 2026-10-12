//! Dedicated synthetic database only; no live service or business project mutations.
#[path = "../../modules/knowledge/core/tests/lifecycle/support.rs"]
mod rules;
#[path = "project_management/support.rs"]
#[allow(dead_code)]
mod support;
use nexofolio_access_contracts::Environments;
use serde_json::json;
#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn postgres_retired_names_can_be_rebuilt_without_losing_history_or_undo() {
    let h = support::Harness::new().await;
    rules::folder_names(h.databases.knowledge.clone()).await;
}

#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn concurrent_same_name_folder_writes_keep_one_effective_sibling() {
    use nexofolio_common::FolderId;
    use nexofolio_contracts::knowledge::*;
    let h = support::Harness::new().await;
    let project = h.project("并发目录名称合成测试").await;
    let k = nexofolio_knowledge::Knowledge::new(
        h.databases.knowledge.clone(),
        nexofolio_observe::Observe::new(h.databases.observe.clone()),
    );
    let author = Author::Person {
        user: h.user("folder-name-writer").await.principal.user_id,
    };
    let a = k.begin(project, author.clone()).await.unwrap();
    let b = k.begin(project, author).await.unwrap();
    let folder = || {
        [Command::PutFolder {
            id: FolderId::new(),
            parent: None,
            name: "同名业务模块".into(),
            summary: None,
            position: 0,
        }]
    };
    let ca = folder();
    let cb = folder();
    let (ra, rb) = tokio::join!(k.apply(a, &ca), k.apply(b, &cb));
    assert!(matches!(
        (&ra, &rb),
        (Ok(()), Err(KnowledgeError::InvalidTree(_)))
            | (Err(KnowledgeError::InvalidTree(_)), Ok(()))
    ));
    assert_eq!(k.catalogue(project).await.unwrap().folders.len(), 1);
}
#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn postgres_lifecycle_projection_and_restart_guards() {
    let h = support::Harness::new().await;
    rules::lifecycle(h.databases.knowledge.clone()).await;
    rules::fields(h.databases.knowledge.clone()).await;
    rules::stale(h.databases.knowledge.clone()).await;
    rules::rollback(h.databases.knowledge.clone()).await;
}
#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn ordinary_member_can_retire_public_reads_mcp_and_recollection_keep_decision() {
    let h = support::Harness::new().await;
    let member = h.user("ordinary-member").await;
    let outsider = h.user("outsider").await;
    let p = h.project("生命周期合成测试").await;
    h.grant(&member, p).await;
    let env = h
        .store
        .create_environment(&member.principal, p, "合成环境")
        .await
        .unwrap();
    let (batch, ids) = h.populate(p, env.id).await;
    let endpoint = ids[0];
    let route = format!("/v1/projects/{p}/document-lifecycle");
    let command = json!({"target":{"kind":"endpoint","id":endpoint},"reason":"合成接口停用","retired":true,"expected_version":0});
    assert_eq!(
        h.call("POST", &route, None, Some(command.clone())).await.0,
        401
    );
    assert_eq!(
        h.call("POST", &route, Some(&outsider.token), Some(command.clone()))
            .await
            .0,
        403
    );
    let (status, state) = h
        .call("POST", &route, Some(&member.token), Some(command.clone()))
        .await;
    assert_eq!(status, 200, "{state}");
    assert_eq!(state["version"], 1);
    assert_eq!(
        h.call("POST", &route, Some(&member.token), Some(command))
            .await
            .0,
        409
    );
    let (_, public) = h
        .call(
            "GET",
            &format!("/v1/public/projects/{p}/document"),
            None,
            None,
        )
        .await;
    assert_eq!(public["lifecycle"]["version"], 1);
    let (_, token) = h
        .databases
        .access
        .issue_mcp_token(member.principal.user_id, 1)
        .await
        .unwrap();
    let active = h
        .mcp(
            &token,
            "search_interfaces",
            json!({"project_id":p,"query":""}),
        )
        .await;
    assert_eq!(active["total"], 1);
    assert_eq!(active["project_lifecycle"]["pending_reconciliation"], true);
    let all = h
        .mcp(
            &token,
            "search_interfaces",
            json!({"project_id":p,"query":"","include_retired":true}),
        )
        .await;
    assert_eq!(all["total"], 2);
    assert_eq!(all["items"][1]["lifecycle"]["state"], "retired");
    let detail = h
        .mcp(
            &token,
            "read_interface",
            json!({"project_id":p,"endpoint_id":endpoint,"section":"overview"}),
        )
        .await;
    assert_eq!(detail["target_lifecycle"]["state"], "retired");
    // Re-send the original batch: observation deduplication never changes lifecycle.
    assert_eq!(
        h.call("POST", "/v1/collect/batches", None, Some(batch))
            .await
            .0,
        200
    );
    assert_eq!(
        h.call("GET", &route, Some(&member.token), None).await.1["version"],
        1
    );
    let undo = json!({"target":{"kind":"endpoint","id":endpoint},"reason":"","retired":false,"expected_version":1});
    assert_eq!(
        h.call("POST", &route, Some(&member.token), Some(undo))
            .await
            .0,
        200
    );
    let active = h
        .mcp(
            &token,
            "search_interfaces",
            json!({"project_id":p,"query":""}),
        )
        .await;
    assert_eq!(active["total"], 2);
    assert_eq!(active["project_lifecycle"]["version"], 2);
}
#[tokio::test]
#[ignore = "requires isolated nexofolio_access_test_* TEST_DATABASE_URL"]
async fn concurrent_decisions_serialize_and_only_one_expected_version_wins() {
    let h = support::Harness::new().await;
    let f = rules::facts();
    let p =
        nexofolio_contracts::endpoint::EndpointReader::list(&f, nexofolio_common::ProjectId::new())
            .await
            .unwrap();
    assert!(p.is_empty());
    let actor = h.user("member-concurrent").await;
    let project = h.project("并发废弃合成资料").await;
    h.grant(&actor, project).await;
    let env = h
        .store
        .create_environment(&actor.principal, project, "test")
        .await
        .unwrap();
    let (_, ids) = h.populate(project, env.id).await;
    let route = format!("/v1/projects/{project}/document-lifecycle");
    let body = |id| {
        Some(
            json!({"target":{"kind":"endpoint","id":id},"reason":"并发","retired":true,"expected_version":0}),
        )
    };
    let (a, b) = tokio::join!(
        h.call("POST", &route, Some(&actor.token), body(ids[0])),
        h.call("POST", &route, Some(&actor.token), body(ids[1]))
    );
    assert!((a.0 == 200 && b.0 == 409) || (a.0 == 409 && b.0 == 200));
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM knowledge.document_lifecycle_history WHERE project_id=$1",
    )
    .bind(support::uuid(project))
    .fetch_one(&h.sql)
    .await
    .unwrap();
    assert_eq!(count, 1);
}
