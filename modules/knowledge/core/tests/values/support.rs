use nexofolio_common::EnvironmentId;
use nexofolio_contracts::{endpoint::*, field_values::*, knowledge::*};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::KnowledgeStore;
use serde_json::{Value, json};
#[path = "../candidates/support.rs"]
#[allow(dead_code)]
mod candidate_support;

pub async fn archive_read_write<S: KnowledgeStore + 'static>(store: S) {
    let facts = candidate_support::facts();
    let project = facts.0.project_id;
    let endpoint = facts.0.id;
    let env = EnvironmentId::new();
    let k = Knowledge::new(store, facts);
    let c = k.create_candidate(project, None, None).await.unwrap();
    let owner = uuid::Uuid::new_v4();
    k.checkpoint_candidate(
        project,
        c.id,
        CandidateState::Running,
        json!({"run_id":owner}),
    )
    .await
    .unwrap();
    let root:Value=serde_json::from_str(r#"[null,"",0,false,{},[],1,"1",123456789012345678901234567890,1.234567890123456789,10,11,12,13,14,15]"#).unwrap();
    let source = EvidenceSource {
        endpoint: Some(endpoint),
        environment: Some(env),
        location: Some(FieldLocation::ResponseBody { status: 200 }),
        example: "fixture".into(),
        complete: Some(true),
        ..Default::default()
    };
    let field = "response_body:200:[]";
    let entries = root
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, v)| ObservedValueRef {
            id: format!("value-{i}"),
            value_type: if v.is_null() {
                ValueType::Null
            } else if v.is_string() {
                ValueType::String
            } else if v.is_boolean() {
                ValueType::Boolean
            } else if v.is_object() {
                ValueType::Object
            } else if v.is_array() {
                ValueType::Array
            } else {
                ValueType::Number
            },
            representation: "json".into(),
            occurrences: vec![ValuePointer {
                document: "doc".into(),
                pointer: format!("/{i}"),
            }],
        })
        .collect();
    let archive = FieldValueArchive {
        version: 1,
        documents: std::collections::BTreeMap::from([(
            "doc".into(),
            ValueDocument {
                source: source.clone(),
                observed_at: None,
                observation_key: None,
                representation: "json".into(),
                state: "full".into(),
                root,
            },
        )]),
        fields: std::collections::BTreeMap::from([(
            format!("{endpoint}/{field}"),
            ObservedField {
                values: entries,
                coverage: vec![ValueCoverage {
                    source,
                    state: "full".into(),
                }],
                hash: "fixture-values".into(),
            },
        )]),
    };
    k.freeze_candidate_values(project, c.id, owner, &archive)
        .await
        .unwrap();
    k.freeze_candidate_values(project, c.id, owner, &archive)
        .await
        .unwrap();
    let q = FieldValueQuery {
        project,
        archive: c.id,
        endpoint,
        field: field.into(),
        offset: 0,
        limit: 12,
        environment: Some(env),
    };
    let first = k.observed_values(q.clone()).await.unwrap();
    assert_eq!(first.items.len(), 12);
    assert_eq!(first.summary.total, 16);
    assert!(first.has_more);
    assert_eq!(first.items[0].preview, "null");
    assert_eq!(first.items[1].preview, "\"\"");
    let next = k
        .observed_values(FieldValueQuery {
            offset: 12,
            ..q.clone()
        })
        .await
        .unwrap();
    assert_eq!(next.items.len(), 4);
    assert!(!next.has_more);
    assert_eq!(
        k.observed_value(q.clone(), "value-8")
            .await
            .unwrap()
            .to_string(),
        "123456789012345678901234567890"
    );
    assert_eq!(
        k.observed_value(q.clone(), "value-9")
            .await
            .unwrap()
            .to_string(),
        "1.234567890123456789"
    );
    assert_eq!(
        k.observed_values(FieldValueQuery {
            environment: Some(EnvironmentId::new()),
            ..q.clone()
        })
        .await
        .unwrap()
        .summary
        .total,
        0
    );
    assert!(
        k.observed_value(
            FieldValueQuery {
                environment: Some(EnvironmentId::new()),
                ..q.clone()
            },
            "value-0"
        )
        .await
        .is_err()
    );
    assert!(
        k.observed_values(FieldValueQuery {
            project: nexofolio_common::ProjectId::new(),
            ..q.clone()
        })
        .await
        .is_err()
    );
    let mut changed = archive;
    changed.documents.get_mut("doc").unwrap().root = json!(["changed"]);
    assert!(
        k.freeze_candidate_values(project, c.id, owner, &changed)
            .await
            .is_err()
    );
    k.checkpoint_candidate(
        project,
        c.id,
        CandidateState::Ready,
        json!({"run_id":owner}),
    )
    .await
    .unwrap();
    k.apply_candidate(project, c.id, nexofolio_common::UserId::new())
        .await
        .unwrap();
    assert_eq!(k.observed_value(q, "value-0").await.unwrap(), Value::Null);
}
