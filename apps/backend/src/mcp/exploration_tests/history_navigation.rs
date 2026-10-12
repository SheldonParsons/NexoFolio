use super::*;
use nexofolio_common::UserId;
use nexofolio_contracts::knowledge::{
    CandidateActor, DocumentLifecycleWriter, LifecycleChange, LifecycleTarget,
};

async fn change(f: &Fixture, target: LifecycleTarget, retired: bool, version: u64) {
    f.knowledge
        .change_lifecycle(
            f.project,
            LifecycleChange {
                target,
                retired,
                expected_version: version,
                reason: "合成目录导航验收".into(),
            },
            CandidateActor::User {
                id: UserId::new(),
                display_name: None,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn active_members_of_retired_directories_remain_discoverable_without_rewriting_history() {
    for retire_root in [true, false] {
        let f = setup().await;
        let retired = if retire_root { f.root } else { f.child };
        change(&f, LifecycleTarget::Folder { id: retired }, true, 0).await;
        let root = call(&f, "browse_directory", json!({"environment_id":f.env}))
            .await
            .unwrap();
        let active_root = root["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["directory_id"] == json!(f.root));
        if retire_root {
            assert!(active_root.is_none());
        } else {
            let row = active_root.unwrap();
            assert_eq!(row["child_directories"], 0);
            assert_eq!(row["subtree_interfaces"], 0);
        }
        let unclassified = call(
            &f,
            "browse_directory",
            root["unclassified"]["arguments"].clone(),
        )
        .await
        .unwrap();
        assert_eq!(unclassified["total"], 1);
        assert_eq!(unclassified["items"][0]["endpoint_id"], json!(f.target));
        assert_eq!(unclassified["items"][0]["lifecycle"]["state"], "active");
        assert_eq!(
            f.knowledge
                .endpoint(f.target)
                .await
                .unwrap()
                .placement
                .unwrap()
                .folder,
            f.child
        );

        // Exact directory reads and history still use the retained placement.
        let exact = call(&f, "browse_directory", json!({"directory_id":f.root,"section":"interfaces","recursive":true,"environment_id":f.env})).await.unwrap();
        assert_eq!(exact["items"][0]["endpoint_id"], json!(f.target));
        let historical_unclassified = call(&f, "browse_directory", json!({"unclassified":true,"section":"interfaces","include_retired":true,"environment_id":f.env})).await.unwrap();
        assert_eq!(historical_unclassified["total"], 0);
        change(&f, LifecycleTarget::Folder { id: retired }, false, 1).await;
        let restored = call(
            &f,
            "browse_directory",
            json!({"unclassified":true,"section":"interfaces","environment_id":f.env}),
        )
        .await
        .unwrap();
        assert_eq!(restored["total"], 0);
    }
}

#[tokio::test]
async fn returned_history_actions_keep_scope_through_directories_interfaces_fields_and_relations() {
    let f = setup().await;
    change(&f, LifecycleTarget::Folder { id: f.root }, true, 0).await;
    change(&f, LifecycleTarget::Endpoint { id: f.target }, true, 1).await;
    let root = call(
        &f,
        "browse_directory",
        json!({"include_retired":true,"environment_id":f.env}),
    )
    .await
    .unwrap();
    let old = root["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["directory_id"] == json!(f.root))
        .unwrap();
    let children = call(&f, "browse_directory", old["open"]["arguments"].clone())
        .await
        .unwrap();
    assert_eq!(children["items"][0]["directory_id"], json!(f.child));
    let interfaces = call(
        &f,
        "browse_directory",
        children["items"][0]["interfaces"]["arguments"].clone(),
    )
    .await
    .unwrap();
    assert_eq!(interfaces["items"][0]["endpoint_id"], json!(f.target));
    let overview = call(
        &f,
        "read_interface",
        interfaces["items"][0]["open"]["arguments"].clone(),
    )
    .await
    .unwrap();
    assert_eq!(overview["target_lifecycle"]["state"], "retired");
    let fields_action = overview["expand"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["arguments"]["section"] == "fields")
        .unwrap();
    let fields = call(&f, "read_interface", fields_action["arguments"].clone())
        .await
        .unwrap();
    assert_eq!(fields["total"], 2);
    let branch = fields["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["children"] == 1)
        .unwrap();
    let leaves = call(&f, "read_interface", branch["expand"]["arguments"].clone())
        .await
        .unwrap();
    assert_eq!(leaves["items"][0]["key"], "request_body:group.name");
    assert_eq!(leaves["items"][0]["lifecycle"]["state"], "retired");
    let relation_action = overview["expand"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["arguments"]["section"] == "relations")
        .unwrap();
    let relations = call(&f, "read_interface", relation_action["arguments"].clone())
        .await
        .unwrap();
    assert_eq!(relations["total"], 1);
    for response in [
        &root,
        &children,
        &interfaces,
        &overview,
        &fields,
        &leaves,
        &relations,
    ] {
        validate_actions(response);
    }
}
