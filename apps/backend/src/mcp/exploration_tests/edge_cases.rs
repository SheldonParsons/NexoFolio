use super::*;
use nexofolio_contracts::{
    endpoint::{EndpointReader, FieldLocation},
    observation::{Body, CanonicalObservation, Content, Fact},
};
#[tokio::test]
async fn changed_sample_content_cannot_be_spliced_into_an_existing_read() {
    let f = setup().await;
    let first = call(
        &f,
        "read_response_material",
        json!({"endpoint_id":f.target,"example_id":"sample","part":"body","max_chars":512}),
    )
    .await
    .unwrap();
    let mut raw: CanonicalObservation = serde_json::from_value(
        f.reader
            .data
            .example(f.target, "sample")
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let Fact::Exchange(exchange) = &mut raw.fact else {
        unreachable!()
    };
    exchange.response.as_mut().unwrap().body = Body::Full {
        media_type: Some("application/json".into()),
        content: Content::Text("{\"changed\":true}".into()),
    };
    f.reader
        .data
        .put_example(f.target, "sample", serde_json::to_value(raw).unwrap());
    assert_eq!(
        call(
            &f,
            "read_response_material",
            first["next"]["arguments"].clone()
        )
        .await
        .unwrap_err(),
        "SOURCE_CHANGED_RESTART_READ"
    );
}
#[tokio::test]
async fn missing_response_and_unretained_context_are_not_invented() {
    let f = setup().await;
    let mut raw: CanonicalObservation = serde_json::from_value(
        f.reader
            .data
            .example(f.target, "sample")
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let Fact::Exchange(exchange) = &mut raw.fact else {
        unreachable!()
    };
    exchange.response = None;
    raw.context = None;
    f.reader
        .data
        .put_example(f.target, "sample", serde_json::to_value(raw).unwrap());
    let response = call(
        &f,
        "read_response_material",
        json!({"endpoint_id":f.target,"example_id":"sample"}),
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(response["data"].as_str().unwrap()).unwrap();
    assert_eq!(value["response_state"], "not_captured");
    assert!(value.get("body").is_none());
    let context = call(
        &f,
        "read_source_evidence",
        json!({"endpoint_id":f.target,"example_id":"sample"}),
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(context["data"].as_str().unwrap()).unwrap();
    assert_eq!(value["context_state"], "not_captured");
}
#[tokio::test]
async fn same_named_fields_keep_request_and_response_relations_separate() {
    let f = setup().await;
    let mut facts = f.reader.data.get(f.target).await.unwrap().unwrap();
    let mut response = facts.fields[0].clone();
    response.location = FieldLocation::ResponseBody { status: 200 };
    facts.fields.push(response);
    f.reader.data.put(facts);
    let result=call(&f,"read_interface",json!({"endpoint_id":f.target,"section":"relations","field_key":"response_body:200:customerId"})).await.unwrap();
    assert_eq!(result["total"], 0);
    let result = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"section":"relations","field_key":"request_body:customerId"}),
    )
    .await
    .unwrap();
    assert_eq!(result["total"], 1);
    assert_eq!(
        call(&f, "browse_directory", json!({"offset":usize::MAX}))
            .await
            .unwrap_err(),
        "INVALID_BOUNDS"
    );
}

#[tokio::test]
async fn explicit_sections_never_silently_ignore_a_field_selector() {
    let f = setup().await;
    assert_eq!(call(&f,"read_interface",json!({"endpoint_id":f.target,"section":"overview","field_key":"request_body:customerId"})).await.unwrap_err(),"SECTION_ARGUMENT_MISMATCH");
    assert_eq!(
        call(
            &f,
            "read_interface",
            json!({"endpoint_id":f.target,"section":"fields","content_offset":100})
        )
        .await
        .unwrap_err(),
        "SECTION_ARGUMENT_MISMATCH"
    );
    let legacy = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"field_offset":1,"limit":1}),
    )
    .await
    .unwrap();
    assert_eq!(legacy["field_offset"], 1);
    assert_eq!(legacy["fields"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn search_does_not_rewrite_a_creation_goal_into_a_saved_submission_name() {
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
            &[Command::PutEndpointNote {
                endpoint: f.target,
                name: Some("保存并提交技术咨询".into()),
                purpose: Some("提交咨询记录".into()),
                context: Default::default(),
            }],
        )
        .await
        .unwrap();
    f.knowledge
        .finish(round, RoundState::Done, None)
        .await
        .unwrap();
    let inferred = call(&f, "search_interfaces", json!({"query":"创建技术咨询"}))
        .await
        .unwrap();
    assert_eq!(inferred["total"], 0);
    let exact = call(
        &f,
        "search_interfaces",
        json!({"query":"保存并提交技术咨询"}),
    )
    .await
    .unwrap();
    assert_eq!(exact["items"][0]["endpoint_id"], json!(f.target));
}

#[tokio::test]
async fn live_call_counters_do_not_force_paging_to_restart() {
    let f = setup().await;
    let first = call(&f, "search_interfaces", json!({"query":"","limit":1}))
        .await
        .unwrap();
    let mut facts = f.reader.data.get(f.target).await.unwrap().unwrap();
    facts.summary.environments[0].calls += 10;
    facts.summary.environments[0].last_seen += chrono::Duration::seconds(30);
    f.reader.data.put(facts);
    let next = call(&f, "search_interfaces", first["next"]["arguments"].clone())
        .await
        .unwrap();
    assert_eq!(next["revision"], first["revision"]);
}
#[tokio::test]
async fn changing_the_open_directory_description_invalidates_its_continuation() {
    let f = setup().await;
    let first = call(
        &f,
        "browse_directory",
        json!({"directory_id":f.root,"limit":1}),
    )
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
                name: "订单".into(),
                summary: Some("更新后的目录说明".into()),
                position: 0,
            }],
        )
        .await
        .unwrap();
    f.knowledge
        .finish(round, RoundState::Done, None)
        .await
        .unwrap();
    let mut args = json!({"directory_id":f.root,"offset":1,"limit":1,"revision":first["revision"]});
    args["project_id"] = json!(f.project);
    assert_eq!(
        call(&f, "browse_directory", args).await.unwrap_err(),
        "SOURCE_CHANGED_RESTART_READ"
    );
}
