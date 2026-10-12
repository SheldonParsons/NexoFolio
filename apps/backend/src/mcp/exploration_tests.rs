//! Agent paths through actual read application boundaries, with synthetic data only.
use super::Navigation;
use nexofolio_common::{EndpointId, EnvironmentId, FolderId, ProjectId};
use serde_json::{Value, json};
mod edge_cases;
mod fixture;
mod history_navigation;
mod lean;
mod lifecycle;
mod navigation_projection;
mod transport;
use fixture::*;
use nexofolio_contracts::knowledge::{
    Author, Command, KnowledgeReader, KnowledgeWriter, RoundState,
};
use std::sync::atomic::Ordering;
#[tokio::test]
async fn directories_are_light_scoped_and_have_an_unclassified_escape() {
    let f = setup().await;
    let root = call(
        &f,
        "browse_directory",
        json!({"environment_id":f.env,"limit":1}),
    )
    .await
    .unwrap();
    assert_eq!(root["items"][0]["name"], "订单");
    assert_eq!(root["items"][0]["subtree_interfaces"], 1);
    assert_eq!(root["has_more"], true);
    assert!(!root.to_string().contains("customerId"));
    let next = &root["next"];
    let second = call(
        &f,
        next["tool"].as_str().unwrap(),
        next["arguments"].clone(),
    )
    .await
    .unwrap();
    assert_eq!(second["items"][0]["name"], "客户");
    let children = call(&f, "browse_directory", json!({"directory_id":f.root}))
        .await
        .unwrap();
    assert_eq!(children["items"][0]["directory_id"], json!(f.child));
    let direct = call(
        &f,
        "browse_directory",
        json!({"directory_id":f.root,"section":"interfaces"}),
    )
    .await
    .unwrap();
    assert_eq!(direct["total"], 0);
    let recursive = call(
        &f,
        "browse_directory",
        json!({"directory_id":f.root,"section":"interfaces","recursive":true}),
    )
    .await
    .unwrap();
    assert_eq!(recursive["items"][0]["endpoint_id"], json!(f.target));
    let unplaced = call(
        &f,
        "browse_directory",
        json!({"section":"interfaces","unclassified":true,"environment_id":f.other_env}),
    )
    .await
    .unwrap();
    assert_eq!(unplaced["items"][0]["endpoint_id"], json!(f.unplaced));
    assert_eq!(
        call(
            &f,
            "browse_directory",
            json!({"directory_id":FolderId::new()})
        )
        .await
        .unwrap_err(),
        "DIRECTORY_NOT_IN_PROJECT"
    );
}
#[tokio::test]
async fn field_text_and_structured_links_both_lead_to_the_related_interface() {
    let f = setup().await;
    f.reader.gets.store(0, Ordering::Relaxed);
    let overview = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"section":"overview"}),
    )
    .await
    .unwrap();
    assert!(!overview.to_string().contains("客户 ID，可从"));
    assert_eq!(
        f.reader.gets.load(Ordering::Relaxed),
        0,
        "overview must not load raw samples"
    );
    assert_eq!(overview["interface"]["field_count"], Value::Null);
    let roots = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"section":"fields"}),
    )
    .await
    .unwrap();
    assert_eq!(roots["total"], 2);
    let child = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"section":"fields","parent_key":"request_body:group"}),
    )
    .await
    .unwrap();
    assert_eq!(child["items"][0]["key"], "request_body:group.name");
    let field = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"section":"field","field_key":"request_body:customerId"}),
    )
    .await
    .unwrap();
    let field: Value = serde_json::from_str(field["data"].as_str().unwrap()).unwrap();
    assert!(
        field["knowledge"]["text"]
            .as_str()
            .unwrap()
            .contains("客户列表查询")
    );
    assert_eq!(field["relations"][0]["confirmation"], "observed");
    assert_eq!(
        field["relations"][0]["related_interface"]["arguments"]["endpoint_id"],
        json!(f.related)
    );
    let related = call(&f, "search_interfaces", json!({"query":"客户列表查询"}))
        .await
        .unwrap();
    assert_eq!(related["items"][0]["endpoint_id"], json!(f.related));
    f.reader.gets.store(0, Ordering::Relaxed);
    let by_field = call(
        &f,
        "search_interfaces",
        json!({"query":"客户编号","search_in":"fields"}),
    )
    .await
    .unwrap();
    assert_eq!(by_field["items"][0]["endpoint_id"], json!(f.target));
    assert_eq!(
        f.reader.gets.load(Ordering::Relaxed),
        0,
        "field search must not load raw endpoint samples"
    );
    let literal = call(&f, "search_interfaces", json!({"query":"创建技术咨询"}))
        .await
        .unwrap();
    assert_eq!(literal["total"], 0, "no scenario-specific synonym guessing");
}
#[tokio::test]
async fn selected_original_parts_reassemble_and_context_is_separate() {
    let f = setup().await;
    let headers = call(
        &f,
        "read_request_material",
        json!({"endpoint_id":f.target,"example_id":"sample","part":"headers"}),
    )
    .await
    .unwrap();
    assert!(!headers["data"].as_str().unwrap().contains("body_root"));
    let mut args =
        json!({"endpoint_id":f.target,"example_id":"sample","part":"body","max_chars":512});
    let mut text = String::new();
    let mut count = 0;
    loop {
        let page = call(&f, "read_response_material", args).await.unwrap();
        text.push_str(page["data"].as_str().unwrap());
        count += 1;
        if page["has_more"] == false {
            break;
        }
        args = page["next"]["arguments"].clone();
        assert!(count < 100);
    }
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        value["body_root"]["data"].as_str().unwrap().chars().count(),
        6000
    );
    assert!(count > 1);
    assert_eq!(value["body_root"]["empty"], Value::Null);
    let context = call(
        &f,
        "read_source_evidence",
        json!({"endpoint_id":f.target,"example_id":"sample"}),
    )
    .await
    .unwrap();
    assert!(context["data"].as_str().unwrap().contains("合成按钮"));
    assert!(!context["data"].as_str().unwrap().contains("字字字"));
    assert_eq!(
        call(
            &f,
            "read_response_material",
            json!({"endpoint_id":f.target,"example_id":"sample","environment_id":f.other_env})
        )
        .await
        .unwrap_err(),
        "ENVIRONMENT_MISMATCH"
    );
}
#[tokio::test]
async fn continuations_reject_changed_sources_and_foreign_scopes() {
    let f = setup().await;
    let first = call(&f, "browse_directory", json!({"limit":1}))
        .await
        .unwrap();
    let round = f
        .knowledge
        .begin(
            f.project,
            Author::Person {
                user: nexofolio_common::UserId::new(),
            },
        )
        .await
        .unwrap();
    f.knowledge
        .apply(
            round,
            &[Command::PutFolder {
                id: f.root,
                parent: None,
                name: "已改名".into(),
                summary: None,
                position: 0,
            }],
        )
        .await
        .unwrap();
    f.knowledge
        .finish(round, RoundState::Done, None)
        .await
        .unwrap();
    assert_eq!(
        call(&f, "browse_directory", first["next"]["arguments"].clone())
            .await
            .unwrap_err(),
        "SOURCE_CHANGED_RESTART_READ"
    );
    assert_eq!(
        call(
            &f,
            "read_interface",
            json!({"endpoint_id":EndpointId::new(),"section":"overview"})
        )
        .await
        .unwrap_err(),
        "ENDPOINT_NOT_IN_PROJECT"
    );
    let denied = f
        .nav
        .call(
            &nexofolio_access_contracts::McpPrincipal {
                user_id: nexofolio_common::UserId::new(),
                token_id: nexofolio_common::TokenId::new(),
            },
            "browse_directory",
            json!({"project_id":ProjectId::new()}),
        )
        .await
        .unwrap_err();
    assert_eq!(denied, "PROJECT_ACCESS_DENIED");
    assert_eq!(
        call(
            &f,
            "search_interfaces",
            json!({"query":"订单","authenticated":true})
        )
        .await
        .unwrap_err(),
        "INVALID_ARGUMENTS"
    );
}
#[tokio::test]
async fn coverage_is_explicit_about_gaps_and_does_not_write() {
    let f = setup().await;
    let before = f.knowledge.rounds(f.project, 100).await.unwrap();
    let out = call(
        &f,
        "inspect_knowledge_coverage",
        json!({"endpoint_id":f.target,"limit":1}),
    )
    .await
    .unwrap();
    assert!(out["total"].as_u64().unwrap() > 0);
    assert!(
        out["boundary"]
            .as_str()
            .unwrap()
            .contains("does not establish completeness")
    );
    assert_eq!(before, f.knowledge.rounds(f.project, 100).await.unwrap());
}
#[test]
fn tool_contracts_are_distinct_and_all_eleven_are_known() {
    let tools = super::tools::definitions();
    assert_eq!(tools.len(), 11);
    for tool in tools {
        let value = serde_json::to_value(tool).unwrap();
        jsonschema::validator_for(&value["inputSchema"]).unwrap();
    }
    assert!(
        super::tools::validate_keys(
            "search_interfaces",
            &json!({"project_id":ProjectId::new(),"section":"fields"})
        )
        .is_err()
    );
}

fn validate_actions(value: &Value) {
    if let Some(object) = value.as_object() {
        if let (Some(tool), Some(args)) = (
            object.get("tool").and_then(Value::as_str),
            object.get("arguments"),
        ) {
            let definition = super::tools::definitions()
                .into_iter()
                .find(|t| t.name == tool)
                .unwrap();
            let schema = serde_json::to_value(definition).unwrap();
            let validator = jsonschema::validator_for(&schema["inputSchema"]).unwrap();
            assert!(
                validator.is_valid(args),
                "invalid next action for {tool}: {args}"
            );
        }
        for v in object.values() {
            validate_actions(v);
        }
    } else if let Some(array) = value.as_array() {
        for v in array {
            validate_actions(v);
        }
    }
}
#[tokio::test]
async fn every_returned_navigation_action_matches_its_own_public_schema() {
    let f = setup().await;
    for (tool, args) in [
        ("browse_directory", json!({"limit":1})),
        (
            "search_interfaces",
            json!({"search_in":"fields","query":"客户"}),
        ),
        (
            "read_interface",
            json!({"endpoint_id":f.target,"section":"overview"}),
        ),
        (
            "read_interface",
            json!({"endpoint_id":f.target,"section":"fields"}),
        ),
        (
            "read_interface",
            json!({"endpoint_id":f.target,"section":"relations"}),
        ),
        (
            "read_interface",
            json!({"endpoint_id":f.target,"section":"samples"}),
        ),
        ("inspect_knowledge_coverage", json!({"limit":1})),
    ] {
        let result = call(&f, tool, args).await.unwrap();
        validate_actions(&result);
    }
    let field = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"section":"field","field_key":"request_body:customerId"}),
    )
    .await
    .unwrap();
    validate_actions(&serde_json::from_str::<Value>(field["data"].as_str().unwrap()).unwrap());
}
