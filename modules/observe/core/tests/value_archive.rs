//! Complete extraction is tested independently from bounded model evidence.
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::{endpoint::*, field_values::*, observation::*, testing::sample_exchange};
use nexofolio_observe::{archive_values, value_field_key};
use serde_json::{Value, json};
fn fixture() -> (EndpointFacts, CanonicalObservation) {
    let project = ProjectId::new();
    let env = EnvironmentId::new();
    (
        EndpointFacts {
            summary: EndpointSummary {
                id: EndpointId::new(),
                project_id: project,
                method: "POST".into(),
                path_template: "/fixture".into(),
                external: false,
                declared: false,
                environments: vec![],
            },
            aliases: vec![],
            addresses: vec![],
            fields: vec![],
            examples: vec![],
            requests: vec![],
        },
        sample_exchange(project, env),
    )
}
fn field(location: FieldLocation, path: Vec<PathSegment>) -> FieldFacts {
    FieldFacts {
        location,
        path: FieldPath(path),
        types: vec![],
        labels: vec![],
        differs_between_environments: false,
        declared: None,
        conflict: None,
    }
}
fn values<'a>(archive: &'a FieldValueArchive, endpoint: EndpointId, key: &str) -> Vec<&'a Value> {
    archive.fields[&value_field_key(endpoint, key)]
        .values
        .iter()
        .map(|v| {
            let p = &v.occurrences[0];
            archive.documents[&p.document]
                .root
                .pointer(&p.pointer)
                .unwrap()
        })
        .collect()
}
#[test]
fn four_locations_preserve_types_empty_values_long_values_duplicates_and_pointers() {
    let (mut facts, mut raw) = fixture();
    let response = FieldLocation::ResponseBody { status: 200 };
    facts.fields = vec![
        field(
            FieldLocation::RequestHeader,
            vec![PathSegment::Key("x-value".into())],
        ),
        field(FieldLocation::RequestBody, vec![]),
        field(
            FieldLocation::RequestBody,
            vec![PathSegment::Key("/~x".into())],
        ),
        field(response, vec![PathSegment::Items]),
        field(
            FieldLocation::ResponseHeader { status: 200 },
            vec![PathSegment::Key("x-value".into())],
        ),
    ];
    let Fact::Exchange(exchange) = &mut raw.fact else {
        unreachable!()
    };
    exchange.request.headers = Some(Headers {
        complete: false,
        entries: vec![
            ("X-Value".into(), "long".repeat(300)),
            ("x-value".into(), "long".repeat(300)),
        ],
    });
    exchange.request.body = Body::Full {
        media_type: Some("application/json".into()),
        content: Content::Text(r#"{"/~x":""}"#.into()),
    };
    exchange.response=Some(HttpResponse{status:200,headers:Some(Headers{complete:true,entries:vec![("X-Value".into(),"".into())]}),body:Body::Full{media_type:Some("application/json".into()),content:Content::Text(r#"[null,"",0,false,{},[],1,"1",123456789012345678901234567890,1.234567890123456789]"#.into())}});
    let archive = archive_values(
        &facts,
        &[("fixture".into(), Some(serde_json::to_value(raw).unwrap()))],
    );
    assert_eq!(
        archive.fields[&value_field_key(facts.summary.id, "request_header:x-value")]
            .values
            .len(),
        1
    );
    assert_eq!(
        archive.fields[&value_field_key(facts.summary.id, "request_header:x-value")].values[0]
            .occurrences
            .len(),
        2
    );
    assert_eq!(
        values(&archive, facts.summary.id, "request_header:x-value")[0]
            .as_str()
            .unwrap()
            .len(),
        1200
    );
    assert_eq!(
        values(&archive, facts.summary.id, "request_body:/~x"),
        vec![&json!("")]
    );
    assert_eq!(
        values(&archive, facts.summary.id, "response_header:200:x-value"),
        vec![&json!("")]
    );
    let observed = values(&archive, facts.summary.id, "response_body:200:[]");
    assert_eq!(observed.len(), 10);
    assert_eq!(observed[0], &Value::Null);
    assert_eq!(observed[1], &json!(""));
    assert_eq!(observed[2], &json!(0));
    assert_eq!(observed[3], &json!(false));
    assert_eq!(observed[4], &json!({}));
    assert_eq!(observed[5], &json!([]));
    assert_ne!(observed[6], observed[7]);
    assert_eq!(observed[8].to_string(), "123456789012345678901234567890");
    assert_eq!(observed[9].to_string(), "1.234567890123456789");
}
#[test]
fn extraction_has_no_twelve_value_or_array_row_cap_and_stores_documents_once() {
    let (mut facts, mut raw) = fixture();
    facts.fields = vec![
        field(FieldLocation::ResponseBody { status: 200 }, vec![]),
        field(
            FieldLocation::ResponseBody { status: 200 },
            vec![PathSegment::Items],
        ),
    ];
    let Fact::Exchange(exchange) = &mut raw.fact else {
        unreachable!()
    };
    exchange.response.as_mut().unwrap().body = Body::Full {
        media_type: Some("application/json".into()),
        content: Content::Text(serde_json::to_string(&(0..5001).collect::<Vec<_>>()).unwrap()),
    };
    let archive = archive_values(
        &facts,
        &[("many".into(), Some(serde_json::to_value(raw).unwrap()))],
    );
    assert_eq!(archive.documents.len(), 1);
    assert_eq!(
        values(&archive, facts.summary.id, "response_body:200:[]").len(),
        5001
    );
    assert_eq!(
        values(&archive, facts.summary.id, "response_body:200:")[0]
            .as_array()
            .unwrap()
            .len(),
        5001
    );
}
#[test]
fn none_unreadable_truncated_and_empty_object_are_separate_from_observed_null() {
    let (mut facts, raw) = fixture();
    facts.fields = vec![field(FieldLocation::RequestBody, vec![])];
    let bodies = [
        Body::None,
        Body::Unreadable {
            media_type: None,
            note: None,
        },
        Body::Full {
            media_type: Some("application/json".into()),
            content: Content::Text("{}".into()),
        },
        Body::Full {
            media_type: Some("application/json".into()),
            content: Content::Text("null".into()),
        },
        Body::Truncated {
            media_type: Some("application/json".into()),
            content: Content::Text("{\"x\":".into()),
            original_bytes: Some(50),
        },
    ];
    let samples = bodies
        .into_iter()
        .enumerate()
        .map(|(i, body)| {
            let mut raw = raw.clone();
            if let Fact::Exchange(e) = &mut raw.fact {
                e.request.body = body;
            }
            (i.to_string(), Some(serde_json::to_value(raw).unwrap()))
        })
        .collect::<Vec<_>>();
    let archive = archive_values(&facts, &samples);
    let v = &archive.fields[&value_field_key(facts.summary.id, "request_body:")];
    assert_eq!(v.values.len(), 3);
    assert_eq!(
        v.coverage
            .iter()
            .map(|c| c.state.as_str())
            .collect::<Vec<_>>(),
        ["none", "unreadable", "full", "full", "truncated"]
    );
}
#[test]
fn form_text_and_file_metadata_remain_usable_and_are_never_file_bytes() {
    let (mut facts, mut raw) = fixture();
    facts.fields = vec![
        field(
            FieldLocation::RequestBody,
            vec![PathSegment::Key("note".into())],
        ),
        field(
            FieldLocation::RequestBody,
            vec![PathSegment::Key("file".into())],
        ),
    ];
    let Fact::Exchange(e) = &mut raw.fact else {
        unreachable!()
    };
    e.request.body=Body::Partial{media_type:Some("multipart/form-data".into()),content:Content::Text(r#"[{"name":"note","value":""},{"name":"file","file":"fixture.txt","type":"text/plain","size":10}]"#.into()),note:None};
    let archive = archive_values(
        &facts,
        &[(
            "form".into(),
            Some(serde_json::to_value(raw.clone()).unwrap()),
        )],
    );
    assert_eq!(
        values(&archive, facts.summary.id, "request_body:note"),
        vec![&json!("")]
    );
    assert_eq!(
        values(&archive, facts.summary.id, "request_body:file")[0]["size"],
        10
    );
    assert!(
        archive
            .documents
            .values()
            .all(|d| d.representation == "multipart_summary" && d.state == "partial")
    );
    if let Fact::Exchange(e) = &mut raw.fact {
        e.request.body = Body::Full {
            media_type: Some("application/x-www-form-urlencoded;charset=UTF-8".into()),
            content: Content::Text("note=&note=a%2Bb&note=%E4%B8%AD%E6%96%87".into()),
        };
    }
    let archive = archive_values(
        &facts,
        &[(
            "urlencoded".into(),
            Some(serde_json::to_value(raw).unwrap()),
        )],
    );
    assert_eq!(
        values(&archive, facts.summary.id, "request_body:note"),
        vec![&json!(""), &json!("a+b"), &json!("中文")]
    );
}
