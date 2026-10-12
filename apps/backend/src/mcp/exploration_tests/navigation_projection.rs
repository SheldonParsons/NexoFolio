//! Navigation reductions preserve identity, scope and executable next reads.
use super::*;
use nexofolio_contracts::endpoint::{EndpointReader, FieldLocation};
use nexofolio_contracts::knowledge::FieldContext;

async fn follow(f: &Fixture, action: &Value) -> Value {
    validate_actions(action);
    call(
        f,
        action["tool"].as_str().unwrap(),
        action["arguments"].clone(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn tree_leaves_read_semantics_and_branches_keep_both_navigation_choices() {
    let f = setup().await;
    let roots = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"environment_id":f.env,"section":"fields"}),
    )
    .await
    .unwrap();
    let rows = roots["items"].as_array().unwrap();
    let leaf = rows
        .iter()
        .find(|r| r["key"] == "request_body:customerId")
        .unwrap();
    let branch = rows
        .iter()
        .find(|r| r["key"] == "request_body:group")
        .unwrap();
    assert_eq!(leaf["children"], 0);
    assert!(leaf.get("expand").is_none());
    assert!(
        leaf.get("path").is_none(),
        "key already retains exact identity"
    );
    assert_eq!(leaf["name"], "客户编号");
    assert_eq!(leaf["related_interfaces"], 1);
    assert_eq!(branch["children"], 1);
    for row in rows {
        assert_eq!(row["open"]["arguments"]["project_id"], json!(f.project));
        assert_eq!(row["open"]["arguments"]["environment_id"], json!(f.env));
        assert_eq!(row["open"]["arguments"]["endpoint_id"], json!(f.target));
        let summary = follow(&f, &row["open"]).await;
        let summary: Value = serde_json::from_str(summary["data"].as_str().unwrap()).unwrap();
        assert_eq!(summary["key"], row["key"]);
        assert!(summary.get("path").is_some());
    }
    let children = follow(&f, &branch["expand"]).await;
    assert_eq!(children["total"], 1);
    assert_eq!(children["items"][0]["key"], "request_body:group.name");
    assert!(children["items"][0].get("expand").is_none());
    assert_eq!(
        call(
            &f,
            "read_interface",
            json!({"endpoint_id":f.target,"section":"fields","parent_key":"request_body:absent"})
        )
        .await
        .unwrap_err(),
        "FIELD_NOT_FOUND"
    );
}

#[tokio::test]
async fn orphan_response_children_remain_discoverable_without_crossing_request_parent() {
    let f = setup().await;
    let mut facts = f.reader.data.get(f.target).await.unwrap().unwrap();
    let mut response = facts.fields[2].clone();
    response.location = FieldLocation::ResponseBody { status: 200 };
    facts.fields.push(response);
    f.reader.data.put(facts);
    let roots = call(&f,"read_interface",json!({"endpoint_id":f.target,"section":"fields","location":{"in":"response_body","status":200}})).await.unwrap();
    assert_eq!(roots["total"], 1);
    assert_eq!(roots["items"][0]["key"], "response_body:200:group.name");
    let summary = follow(&f, &roots["items"][0]["open"]).await;
    let summary: Value = serde_json::from_str(summary["data"].as_str().unwrap()).unwrap();
    assert_eq!(summary["location"]["in"], "response_body");
    assert_eq!(summary["location"]["status"], 200);
    let children = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"section":"fields","parent_key":"request_body:group"}),
    )
    .await
    .unwrap();
    assert_eq!(children["total"], 1);
    assert_eq!(children["items"][0]["key"], "request_body:group.name");
}

#[tokio::test]
async fn field_search_opens_complete_semantics_without_loading_overview_or_samples() {
    let f = setup().await;
    f.reader.gets.store(0, Ordering::Relaxed);
    let hits = call(
        &f,
        "search_interfaces",
        json!({"environment_id":f.env,"search_in":"fields","query":"客户编号"}),
    )
    .await
    .unwrap();
    assert_eq!(f.reader.gets.load(Ordering::Relaxed), 0);
    assert_eq!(hits["total"], 1);
    let row = &hits["items"][0];
    for key in ["method", "path", "name", "purpose", "directory_path"] {
        assert!(row.get(key).is_some(), "owner context must remain: {key}");
    }
    assert!(row.get("open").is_none());
    assert!(row.get("environments").is_none());
    assert_eq!(row["matched_field"]["name"], "客户编号");
    assert_eq!(row["matched_field"]["identity_state"], "qualified");
    let summary = follow(&f, &row["matched_field"]["open"]).await;
    let summary: Value = serde_json::from_str(summary["data"].as_str().unwrap()).unwrap();
    assert_eq!(summary["key"], row["matched_field"]["key"]);
    assert!(
        summary["meaning"]["text"]
            .as_str()
            .unwrap()
            .contains("客户列表查询")
    );
    // Interface mode retains its original overview action and traffic metadata.
    let interfaces = call(&f, "search_interfaces", json!({"query":"/orders"}))
        .await
        .unwrap();
    assert!(interfaces["items"][0].get("environments").is_some());
    let overview = follow(&f, &interfaces["items"][0]["open"]).await;
    assert_eq!(overview["interface"]["facts"]["id"], json!(f.target));
}

#[tokio::test]
async fn unqualified_field_search_still_expands_facts_instead_of_guessing_location() {
    let f = setup().await;
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
            &[Command::PutFieldNote {
                endpoint: f.target,
                path: "legacyClue".into(),
                text: "仅有旧格式文字线索".into(),
                values: vec![],
                context: FieldContext::default(),
            }],
        )
        .await
        .unwrap();
    f.knowledge
        .finish(round, RoundState::Done, None)
        .await
        .unwrap();
    let hits = call(
        &f,
        "search_interfaces",
        json!({"search_in":"fields","query":"旧格式文字线索"}),
    )
    .await
    .unwrap();
    let field = &hits["items"][0]["matched_field"];
    assert_eq!(field["identity_state"], "legacy_location_unknown");
    assert_eq!(field["open"]["arguments"]["section"], "fields");
    assert!(field["open"]["arguments"].get("field_key").is_none());
    let fields = follow(&f, &field["open"]).await;
    assert_eq!(fields["total"], 2);
}

#[test]
fn advertised_defaults_and_parameter_guidance_match_existing_bounds() {
    for tool in super::super::tools::definitions() {
        let value = serde_json::to_value(tool).unwrap();
        let schema = &value["inputSchema"];
        if let Some(limit) = schema["properties"].get("limit") {
            assert_eq!(
                (
                    limit["minimum"].as_u64(),
                    limit["maximum"].as_u64(),
                    limit["default"].as_u64()
                ),
                (Some(1), Some(50), Some(20))
            );
            assert!(
                limit["description"]
                    .as_str()
                    .unwrap()
                    .contains("100 is invalid")
            );
        }
        if let Some(chars) = schema["properties"].get("max_chars") {
            assert_eq!(chars["maximum"], 16384);
            assert_eq!(chars["default"], 16384);
            assert!(chars["description"].as_str().unwrap().contains("Unicode"));
        }
    }
    let tool = super::super::tools::definitions()
        .into_iter()
        .find(|t| t.name == "list_projects")
        .unwrap();
    let value = serde_json::to_value(tool).unwrap();
    let validator = jsonschema::validator_for(&value["inputSchema"]).unwrap();
    assert!(validator.is_valid(&json!({})));
    assert!(validator.is_valid(&json!({"limit":50})));
    assert!(!validator.is_valid(&json!({"limit":100})));
}
