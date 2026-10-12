//! Regression cases from the user's retrieval trace; all records are synthetic.
use super::*;
use nexofolio_access_contracts::McpPrincipal;
use nexofolio_common::{TokenId, UserId};
use nexofolio_contracts::{endpoint::*, knowledge::*};

async fn document(f: &Fixture, tool: &str, mut args: Value) -> (Value, usize) {
    let mut text = String::new();
    let mut expected = None;
    loop {
        let out = call(f, tool, args).await.unwrap();
        let rev = out["revision"].as_str().unwrap();
        if let Some(previous) = &expected {
            assert_eq!(previous, rev);
        } else {
            expected = Some(rev.to_owned());
        }
        text.push_str(out["data"].as_str().unwrap());
        if out["has_more"] == false {
            assert_eq!(
                text.chars().count() as u64,
                out["total_chars"].as_u64().unwrap()
            );
            break;
        }
        args = out["next"]["arguments"].clone();
    }
    let size = text.chars().count();
    (serde_json::from_str(&text).unwrap(), size)
}
fn source(endpoint: EndpointId, env: EnvironmentId, location: FieldLocation) -> EvidenceSource {
    EvidenceSource {
        endpoint: Some(endpoint),
        example: "sample".into(),
        environment: Some(env),
        location: Some(location),
        pointer: Some("/data/id".into()),
        page: None,
        sequence: None,
        platform: None,
        complete: Some(true),
    }
}
#[tokio::test]
async fn field_semantics_remain_complete_while_large_evidence_is_deferred() {
    let f = setup().await;
    let text = format!(
        "{}末尾线索：请查询客户列表查询接口。",
        "完整业务说明".repeat(120)
    );
    let sources = vec![
        source(
            f.related,
            f.env,
            FieldLocation::ResponseBody { status: 200 },
        ),
        source(f.target, f.env, FieldLocation::RequestBody),
    ];
    let round = f
        .knowledge
        .begin(
            f.project,
            Author::Person {
                user: UserId::new(),
            },
        )
        .await
        .unwrap();
    let evidence: Vec<_> = (0..12)
        .map(|_| FieldEvidence {
            kind: EvidenceKind::RelatedInterface,
            text: "原始证据内容".repeat(100),
            endpoint: f.related,
            field: "data[].id".into(),
            example: Some("sample".into()),
            environment: Some(f.env),
            options: (0..30)
                .map(|n| FieldOption {
                    value: format!("synthetic-{n}"),
                    label: Some("原始选项标签".repeat(5)),
                })
                .collect(),
            source: Some(sources[0].clone()),
        })
        .collect();
    f.knowledge
        .apply(
            round,
            &[
                Command::PutFieldNote {
                    endpoint: f.target,
                    path: "request_body:customerId".into(),
                    text: text.clone(),
                    values: vec!["legacy-preview".into()],
                    context: FieldContext {
                        location: Some(FieldLocation::RequestBody),
                        field_path: Some("customerId".into()),
                        display_name: Some("客户编号".into()),
                        meaning: FieldMeaning {
                            unknowns: vec!["选值规则待核实".into()],
                            ..Default::default()
                        },
                        evidence: evidence.clone(),
                        ..Default::default()
                    },
                },
                Command::PutLink {
                    from: f.related,
                    from_field: Some("data[].id".into()),
                    to: f.target,
                    to_field: Some("customerId".into()),
                    relation: Relation::ParameterSource,
                    evidence: "合成值对应".into(),
                    context: LinkContext {
                        from_location: Some(FieldLocation::ResponseBody { status: 200 }),
                        to_location: Some(FieldLocation::RequestBody),
                        environment: Some(f.env),
                        confirmation: Confirmation::Observed,
                        sources,
                        ..Default::default()
                    },
                },
            ],
        )
        .await
        .unwrap();
    f.knowledge
        .finish(round, RoundState::Done, None)
        .await
        .unwrap();
    let args = json!({"endpoint_id":f.target,"field_key":"request_body:customerId","section":"field_summary"});
    let (summary, small) = document(&f, "read_interface", args).await;
    let (full, large) = document(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"field_key":"request_body:customerId","section":"field"}),
    )
    .await;
    assert_eq!(summary["meaning"]["text"], text);
    assert_eq!(
        summary["meaning"]["business_meaning"]["unknowns"][0],
        "选值规则待核实"
    );
    assert_eq!(
        summary["meaning"]["business_meaning"]["confirmation"],
        "candidate"
    );
    assert_eq!(summary["relations"]["items"][0]["confirmation"], "observed");
    assert_eq!(summary["evidence"]["total"], 12);
    assert_eq!(
        summary["related_evidence"]["total"], 0,
        "do not repeat the same related interface"
    );
    assert!(small < large / 2, "summary={small}, full={large}");
    assert_eq!(
        full["knowledge"]["context"]["evidence"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
    assert_eq!(summary["legacy_value_preview"]["count"], 1);
    assert_eq!(summary["text_search"]["required_input"], "query");
    assert!(summary["text_search"].get("arguments").is_none());
    validate_actions(&summary);
    let action = &summary["evidence"]["open"];
    let (stored, _) = document(
        &f,
        action["tool"].as_str().unwrap(),
        action["arguments"].clone(),
    )
    .await;
    assert_eq!(stored["evidence"].as_array().unwrap().len(), 12);
    assert_eq!(
        stored["evidence"][0]["options"].as_array().unwrap().len(),
        30
    );
    let related = &summary["relations"]["items"][0]["evidence"];
    f.reader.gets.store(0, Ordering::Relaxed);
    let (record, _) = document(
        &f,
        related["tool"].as_str().unwrap(),
        related["arguments"].clone(),
    )
    .await;
    assert_eq!(
        f.reader.gets.load(Ordering::Relaxed),
        0,
        "source actions must not prefetch samples"
    );
    assert_eq!(record["source_reads"].as_array().unwrap().len(), 2);
    validate_actions(&record);
    for next in record["source_reads"].as_array().unwrap() {
        let action = &next["material"];
        let (body, _) = document(
            &f,
            action["tool"].as_str().unwrap(),
            action["arguments"].clone(),
        )
        .await;
        assert!(body.get("body").is_some());
    }
    eprintln!("synthetic_field_summary_chars={small}; legacy_full_field_chars={large}");
}
#[tokio::test]
async fn coverage_summarizes_59_candidates_without_listing_them_or_marking_them_failed() {
    let f = setup().await;
    let mut facts = f.reader.data.get(f.target).await.unwrap().unwrap();
    facts.fields = (0..59)
        .map(|n| fixture::field(vec![PathSegment::Key(format!("f{n}"))]))
        .collect();
    f.reader.data.put(facts);
    let round = f
        .knowledge
        .begin(
            f.project,
            Author::Person {
                user: UserId::new(),
            },
        )
        .await
        .unwrap();
    let commands: Vec<_> = (0..59)
        .map(|n| Command::PutFieldNote {
            endpoint: f.target,
            path: format!("request_body:f{n}"),
            text: "已有完整说明，业务含义仍需核实".into(),
            values: vec![],
            context: FieldContext {
                location: Some(FieldLocation::RequestBody),
                field_path: Some(format!("f{n}")),
                display_name: Some(format!("字段{n}")),
                ..Default::default()
            },
        })
        .collect();
    f.knowledge.apply(round, &commands).await.unwrap();
    f.knowledge
        .finish(round, RoundState::Done, None)
        .await
        .unwrap();
    let result = call(
        &f,
        "inspect_knowledge_coverage",
        json!({"endpoint_id":f.target,"section":"summary"}),
    )
    .await
    .unwrap();
    assert_eq!(result["coverage"]["fields"], 59);
    assert_eq!(result["coverage"]["generation_addressed"], 59);
    assert_eq!(result["coverage"]["semantic_confirmation"]["candidate"], 59);
    assert!(result.get("items").is_none());
    assert!(result["groups"].as_array().unwrap().len() < 8);
    validate_actions(&result);
    let group = result["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["kind"] == "field_meaning_unconfirmed")
        .unwrap();
    assert_eq!(group["count"], 59);
    let page = call(
        &f,
        "inspect_knowledge_coverage",
        group["open"]["arguments"].clone(),
    )
    .await
    .unwrap();
    assert_eq!(page["total"], 59);
    assert_eq!(page["items"].as_array().unwrap().len(), 20);
    let single = call(
        &f,
        "inspect_knowledge_coverage",
        json!({"endpoint_id":f.target,"section":"summary","field_key":"request_body:f0"}),
    )
    .await
    .unwrap();
    assert_eq!(single["coverage"]["fields"], 1);
    assert_eq!(single["coverage"]["capture_inspected"], false);
    assert_eq!(single["issue_records"], 1);
    let legacy = call(
        &f,
        "inspect_knowledge_coverage",
        json!({"endpoint_id":f.target,"limit":1}),
    )
    .await
    .unwrap();
    assert!(legacy.get("items").is_some());
    assert_eq!(legacy["total"], 60);
    assert_eq!(
        call(
            &f,
            "inspect_knowledge_coverage",
            json!({"endpoint_id":f.target,"section":"summary","field_key":"request_body:not_found"})
        )
        .await
        .unwrap_err(),
        "FIELD_NOT_FOUND"
    );
}
#[tokio::test]
async fn invalid_bounds_return_repair_details_without_echoing_input_content() {
    let f = setup().await;
    let error = call(
        &f,
        "read_source_evidence",
        json!({"endpoint_id":f.target,"max_chars":20000}),
    )
    .await
    .unwrap_err()
    .payload();
    assert_eq!(error["error"], "INVALID_BOUNDS");
    assert_eq!(error["details"]["parameter"], "max_chars");
    assert_eq!(error["details"]["maximum"], 16384);
    assert_eq!(error["details"]["suggested"], 16384);
    let long = "private-input".repeat(30);
    let error = call(&f, "search_interfaces", json!({"query":long}))
        .await
        .unwrap_err()
        .payload();
    assert!(!error.to_string().contains("private-input"));
    assert_eq!(error["details"]["parameter"], "query");
    assert_eq!(error["details"]["constraint"], "string_length");
    assert!(error["details"].get("suggested").is_none());
    let p = McpPrincipal {
        user_id: UserId::new(),
        token_id: TokenId::new(),
    };
    let error = f
        .nav
        .call(&p, "list_projects", json!({"limit":51}))
        .await
        .unwrap_err()
        .payload();
    assert_eq!(error["details"]["parameter"], "limit");
}

#[tokio::test]
async fn a_new_issue_filter_opts_in_without_changing_legacy_pages() {
    let f = setup().await;
    let summary = call(
        &f,
        "inspect_knowledge_coverage",
        json!({"endpoint_id":f.target,"section":"summary"}),
    )
    .await
    .unwrap();
    assert_eq!(summary["coverage"]["generation_addressed"], 1);
    assert_eq!(summary["coverage"]["generation_not_addressed"], 2);
    let filtered = call(
        &f,
        "inspect_knowledge_coverage",
        json!({"endpoint_id":f.target,"issue_kind":"field_generation_incomplete"}),
    )
    .await
    .unwrap();
    assert_eq!(filtered["total"], 2);
    let legacy = call(
        &f,
        "inspect_knowledge_coverage",
        json!({"endpoint_id":f.target}),
    )
    .await
    .unwrap();
    assert_eq!(legacy["total"], 4);
}
