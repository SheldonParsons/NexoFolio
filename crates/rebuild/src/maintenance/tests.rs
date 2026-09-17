use super::*;
fn snapshot() -> KnowledgeSnapshot {
    let interface = CatalogInterface {
        interface_id: InterfaceId::new(),
        method: "GET".into(),
        path: "/orders/{param1}".into(),
        recognized_path: None,
        environments: vec![CatalogEnvironment {
            environment_id: EnvironmentId::new(),
            environment_name: "test".into(),
            revision_id: RevisionId::new(),
            definition: json!({"request":{"parameters":[{"in":"path","name":"param1","observed_schema":{"type":"string"}}],"headers":[{"name":"x-request-id","observed_schema":{"type":"string"}}],"body":{"observed_schema":{"type":"object","properties":{"status":{"type":"number"},"items":{"type":"array","items":{"unknown":true}}}}}},"response":{"headers":[],"body":{"observed_schema":{"type":"object","properties":{"id":{"type":"number"}}}}}}),
        }],
    };
    let fields = snapshot_fields(std::slice::from_ref(&interface), &[]);
    KnowledgeSnapshot {
        id: Uuid::new_v4(),
        project_id: ProjectId::new(),
        base_generation: 0,
        base_catalog_version: None,
        base_knowledge_version: None,
        system_directory_id: DirectoryId::new(),
        catalog: DirectoryCandidate {
            nodes: vec![],
            assignments: vec![PreviewAssignment {
                interface_id: interface.interface_id,
                directory_id: None,
                reason: "new".into(),
            }],
            merge_groups: vec![],
        },
        interfaces: vec![interface],
        fields,
        annotations: vec![],
        facts: vec![],
        pending_observations: 0,
        directory_metrics: None,
        previous_maintenance: None,
        inputs: Some(SnapshotInputs::default()),
    }
}
fn coverage(s: &KnowledgeSnapshot) -> ReviewCoverage {
    ReviewCoverage {
        total_interfaces: s.interfaces.len(),
        total_fields: s.fields.len(),
        reviewed_fields: s.fields.len(),
        total_segments: 1,
        completed_segments: 1,
        complete: true,
    }
}
fn keep() -> MaintenancePlan {
    MaintenancePlan {
        strategy: MaintenanceStrategy::Keep,
        reason: "current structure adequate".into(),
        expected_benefit: "preserve".into(),
        actions: vec![],
    }
}
#[test]
fn every_field_pointer_resolves_in_original_definition() {
    let s = snapshot();
    for f in &s.fields {
        for p in &f.schema_pointers {
            assert!(
                s.interfaces[0].environments[0]
                    .definition
                    .pointer(p)
                    .is_some(),
                "{p}"
            );
        }
    }
}
#[test]
fn review_requires_every_unit_exactly_once() {
    let mut s = snapshot();
    s.fields[0].schema = json!({"properties":(0..300).map(|n|(format!("p{n}"),json!({"type":"string"}))).collect::<serde_json::Map<_,_>>()});
    let segments = review_segments(&s, 4096).unwrap();
    assert!(segments.len() > 1);
    let all: HashSet<_> = segments
        .iter()
        .flat_map(|s| s.units.iter().map(|u| u.field_id.clone()))
        .collect();
    assert_eq!(all.len(), s.fields.len());
    let segment = &segments[0];
    let mut review = SegmentReview {
        segment_id: segment.id.clone(),
        summary: "read".into(),
        assessments: segment
            .units
            .iter()
            .map(|u| FieldAssessment {
                unit_id: u.id.clone(),
                field_id: u.field_id.clone(),
                disposition: "keep".into(),
                note: "read".into(),
                evidence: vec![],
            })
            .collect(),
    };
    validate_segment(segment, &review).unwrap();
    review.assessments.pop();
    assert!(validate_segment(segment, &review).is_err());
    review.assessments = segment
        .units
        .iter()
        .map(|u| FieldAssessment {
            unit_id: u.id.clone(),
            field_id: u.field_id.clone(),
            disposition: "keep".into(),
            note: "read".into(),
            evidence: vec![],
        })
        .collect();
    review.assessments.push(review.assessments[0].clone());
    assert!(validate_segment(segment, &review).is_err());
}
#[test]
fn incomplete_review_and_system_mutation_cannot_materialize() {
    let s = snapshot();
    let mut c = coverage(&s);
    c.complete = false;
    assert!(materialize_maintenance(&s, &keep(), Uuid::new_v4(), c).is_err());
    let mut plan = keep();
    plan.actions.push(MaintenanceAction::SetDirectory {
        node: PreviewNode {
            id: s.system_directory_id,
            parent: None,
            name: "renamed".into(),
            description: "".into(),
        },
        reason: "model request".into(),
        evidence: vec![KnowledgeEvidenceRef {
            kind: "field".into(),
            id: s.fields[0].id.clone(),
        }],
    });
    assert!(materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s)).is_err());
}
#[test]
fn enums_require_typed_field_values_and_real_label_evidence() {
    let mut s = snapshot();
    let f = s
        .fields
        .iter()
        .find(|f| f.reference.path == "/status")
        .unwrap()
        .clone();
    let fact = EvidenceFact {
        id: Uuid::new_v4(),
        project_id: s.project_id,
        environment_id: f.reference.environment_id,
        kind: "observed_value".into(),
        subject: serde_json::to_value(&f.reference).unwrap(),
        data: json!({"state":"present","value":1,"complete_enum":false}),
        observations: 9,
        first_seen: "2026-09-15T00:00:00Z".into(),
        last_seen: "2026-09-15T00:01:00Z".into(),
        samples: vec![],
    };
    let evidence = vec![KnowledgeEvidenceRef {
        kind: "fact".into(),
        id: fact.id.to_string(),
    }];
    s.facts.push(fact);
    let plan = |value: Value, label: Option<String>, complete: bool| {
        let mut p = keep();
        p.actions.push(MaintenanceAction::UpsertAnnotation {
            reason: "sample".into(),
            annotation: Box::new(AnnotationDraft {
                id: Uuid::new_v4(),
                target: SemanticTarget::Field {
                    field: f.reference.adopted().unwrap(),
                },
                value: SemanticValue::Enum {
                    entries: vec![EnumEntry {
                        state: UiValueState::Present,
                        value,
                        label,
                    }],
                    complete,
                    scope: json!({}),
                },
                evidence: evidence.clone(),
                verification: Verification::Observed,
                note: "sample only".into(),
            }),
        });
        p
    };
    assert!(
        materialize_maintenance(
            &s,
            &plan(json!(1), None, false),
            Uuid::new_v4(),
            coverage(&s)
        )
        .is_ok()
    );
    for p in [
        plan(json!("1"), None, false),
        plan(json!(1), Some("invented".into()), false),
        plan(json!(1), None, true),
    ] {
        assert!(materialize_maintenance(&s, &p, Uuid::new_v4(), coverage(&s)).is_err());
    }
}
#[test]
fn missing_recent_values_cannot_remove_published_enum_values() {
    let mut s = snapshot();
    let field = s
        .fields
        .iter()
        .find(|f| f.reference.path == "/status")
        .unwrap()
        .reference
        .clone();
    let fact_id = Uuid::new_v4();
    s.facts.push(EvidenceFact {
        id: fact_id,
        project_id: s.project_id,
        environment_id: field.environment_id,
        kind: "observed_value".into(),
        subject: serde_json::to_value(&field).unwrap(),
        data: json!({"state":"present","value":1,"complete_enum":false}),
        observations: 1,
        first_seen: "2026-09-15T00:00:00Z".into(),
        last_seen: "2026-09-15T00:00:00Z".into(),
        samples: vec![],
    });
    let evidence = vec![KnowledgeEvidenceRef {
        kind: "fact".into(),
        id: fact_id.to_string(),
    }];
    let id = Uuid::new_v4();
    let mut old = AnnotationDraft {
        id,
        target: SemanticTarget::Field {
            field: field.adopted().unwrap(),
        },
        value: SemanticValue::Enum {
            entries: vec![1, 2]
                .into_iter()
                .map(|v| EnumEntry {
                    state: UiValueState::Present,
                    value: json!(v),
                    label: None,
                })
                .collect(),
            complete: false,
            scope: json!({}),
        },
        evidence: evidence.clone(),
        verification: Verification::Observed,
        note: "observed history".into(),
    };
    s.annotations.push(SemanticAnnotation {
        annotation: old.clone(),
        basis: vec![DefinitionBasis {
            interface_id: field.interface_id,
            environment_id: field.environment_id,
            revision_id: field.revision_id.unwrap(),
        }],
        source_run_id: Uuid::new_v4(),
        stale: false,
    });
    if let SemanticValue::Enum { entries, .. } = &mut old.value {
        entries.pop();
    }
    let mut plan = keep();
    plan.actions.push(MaintenanceAction::UpsertAnnotation {
        annotation: Box::new(old),
        reason: "Only 1 was seen recently".into(),
    });
    assert!(
        materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s))
            .unwrap_err()
            .to_string()
            .contains("ENUM_REMOVAL_REQUIRES_COUNTEREVIDENCE")
    );
    plan.actions = vec![MaintenanceAction::RetractAnnotation {
        annotation_id: id,
        reason: "Not seen recently".into(),
        evidence,
    }];
    assert!(
        materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s))
            .unwrap_err()
            .to_string()
            .contains("ENUM_RETRACTION_REQUIRES_COUNTEREVIDENCE")
    );
}
#[test]
fn field_review_includes_evidence_and_points_to_stale_annotations_without_migrating() {
    let mut s = snapshot();
    let field = s
        .fields
        .iter()
        .find(|f| f.reference.path == "/status")
        .unwrap()
        .reference
        .clone();
    let evidence_id = Uuid::new_v4();
    s.facts.push(EvidenceFact {
        id: evidence_id,
        project_id: s.project_id,
        environment_id: field.environment_id,
        kind: "enum_label_candidate".into(),
        subject: serde_json::to_value(&field).unwrap(),
        data: json!({"state":"present","value":1,"label":"待审核","complete_enum":false}),
        observations: 1,
        first_seen: "2026-09-15T00:00:00Z".into(),
        last_seen: "2026-09-15T00:00:00Z".into(),
        samples: vec![],
    });
    let mut old_field = field.clone();
    old_field.revision_id = Some(RevisionId::new());
    let old_id = Uuid::new_v4();
    s.annotations.push(SemanticAnnotation {
        annotation: AnnotationDraft {
            id: old_id,
            target: SemanticTarget::Field {
                field: old_field.adopted().unwrap(),
            },
            value: SemanticValue::Description {
                text: "旧说明".into(),
            },
            evidence: vec![],
            verification: Verification::Inferred,
            note: "old".into(),
        },
        basis: vec![],
        source_run_id: Uuid::new_v4(),
        stale: true,
    });
    let segments = review_segments(&s, 19000).unwrap();
    let unit = segments
        .iter()
        .flat_map(|s| &s.units)
        .find(|u| u.reference == field)
        .unwrap();
    assert_eq!(
        unit.context["evidence_summary"]["kinds"]["enum_label_candidate"],
        1
    );
    assert_eq!(
        unit.context["evidence_summary"]["representatives"][0]["reference"]["id"],
        evidence_id.to_string()
    );
    assert_eq!(
        unit.context["stale_annotation_refs"][0]["id"],
        old_id.to_string()
    );
    assert!(unit.annotations.is_empty());
}

#[test]
fn review_packs_complete_interface_by_bytes_instead_of_six_units() {
    let mut s = snapshot();
    s.interfaces[0].environments[0].definition["response"]["body"]["observed_schema"] = json!({
        "type":"object", "properties":(0..40).map(|n|(format!("field{n}"),json!({"type":"string"}))).collect::<serde_json::Map<_,_>>()
    });
    s.fields = snapshot_fields(&s.interfaces, &[]);
    let segments = review_segments(&s, 128 * 1024).unwrap();
    assert_eq!(
        segments.len(),
        1,
        "an interface that fits should be read together"
    );
    assert!(segments[0].units.len() > 6);
    let covered: HashSet<_> = segments[0]
        .units
        .iter()
        .map(|u| u.field_id.clone())
        .collect();
    assert_eq!(covered.len(), s.fields.len());
}

#[test]
fn strategies_have_explicit_directory_boundaries() {
    let mut s = snapshot();
    let node = PreviewNode {
        id: DirectoryId::new(),
        parent: None,
        name: "existing".into(),
        description: String::new(),
    };
    s.catalog.nodes.push(node.clone());
    s.catalog.assignments[0].directory_id = Some(node.id);
    let basis = KnowledgeEvidenceRef {
        kind: "interface".into(),
        id: s.interfaces[0].interface_id.to_string(),
    };
    let mut renamed = node.clone();
    renamed.name = "renamed".into();
    let action = MaintenanceAction::SetDirectory {
        node: renamed,
        reason: "rename".into(),
        evidence: vec![basis.clone()],
    };
    let mut plan = MaintenancePlan {
        strategy: MaintenanceStrategy::Insert,
        reason: "test".into(),
        expected_benefit: "test".into(),
        actions: vec![action.clone()],
    };
    assert!(materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s)).is_err());
    plan.strategy = MaintenanceStrategy::Partial;
    assert!(materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s)).is_ok());
    plan.strategy = MaintenanceStrategy::Keep;
    assert!(materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s)).is_err());
    plan.strategy = MaintenanceStrategy::Insert;
    plan.actions = vec![MaintenanceAction::AssignInterface {
        interface_id: s.interfaces[0].interface_id,
        directory_id: None,
        reason: "move".into(),
        evidence: vec![basis.clone()],
    }];
    assert!(materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s)).is_err());
    s.catalog.assignments[0].directory_id = None;
    plan.actions = vec![MaintenanceAction::AssignInterface {
        interface_id: s.interfaces[0].interface_id,
        directory_id: Some(node.id),
        reason: "insert".into(),
        evidence: vec![basis.clone()],
    }];
    assert!(materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s)).is_ok());
    plan.actions = vec![MaintenanceAction::ReplaceCatalog {
        candidate: s.catalog.clone(),
        reason: "replace".into(),
        evidence: vec![basis],
    }];
    plan.strategy = MaintenanceStrategy::Partial;
    assert!(materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s)).is_err());
    plan.strategy = MaintenanceStrategy::Full;
    assert!(materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s)).is_ok());
}

#[test]
fn keep_can_update_semantics_without_changing_the_catalog() {
    let s = snapshot();
    let interface = s.interfaces[0].interface_id;
    let mut plan = keep();
    plan.actions.push(MaintenanceAction::UpsertAnnotation {
        annotation: Box::new(AnnotationDraft {
            id: Uuid::new_v4(),
            target: SemanticTarget::Interface {
                interface_id: interface,
            },
            value: SemanticValue::Description {
                text: "Observed order lookup".into(),
            },
            evidence: vec![KnowledgeEvidenceRef {
                kind: "interface".into(),
                id: interface.to_string(),
            }],
            verification: Verification::Inferred,
            note: "fixture".into(),
        }),
        reason: "clarify description".into(),
    });
    let result = materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s)).unwrap();
    assert_eq!(
        serde_json::to_value(result.catalog).unwrap(),
        serde_json::to_value(s.catalog).unwrap()
    );
    assert_eq!(result.annotations.len(), 1);
}

#[test]
fn observation_fields_share_review_but_cannot_be_adopted_by_annotations() {
    let mut s = snapshot();
    let i = &s.interfaces[0];
    let e = &i.environments[0];
    let formal = FieldRef {
        interface_id: i.interface_id,
        environment_id: e.environment_id,
        revision_id: e.revision_id,
        location: "request.body".into(),
        path: "/new_field".into(),
    };
    let id = Uuid::new_v4();
    let mut definition = e.definition.clone();
    definition["request"]["body"]["observed_schema"]["properties"]["new_field"] =
        json!({"type":"number"});
    s.inputs
        .as_mut()
        .unwrap()
        .observations
        .push(SnapshotObservation {
            record: ObservationAssessment {
                project_id: s.project_id,
                interface_id: i.interface_id,
                environment_id: e.environment_id,
                ingestion_id: id,
                base_revision_id: Some(e.revision_id),
                extractor_version: Some("observed-http-2".into()),
                assessment: None,
                incoming_definition: Some(definition),
            },
            reconstructed_definition: None,
        });
    let adopted_before: Vec<_> = s.fields.iter().map(|f| f.id.clone()).collect();
    s.fields = complete_snapshot_fields(&s);
    assert!(
        adopted_before
            .iter()
            .all(|id| s.fields.iter().any(|f| &f.id == id))
    );
    let observed = s
        .fields
        .iter()
        .find(|f| f.reference.path == "/new_field")
        .unwrap();
    assert!(observed.reference.adopted().is_none());
    assert_eq!(
        observed.reference.observation.as_ref().unwrap().path,
        "/new_field"
    );
    assert_eq!(review_field_context(&s, observed)["writable"], false);
    assert!(
        field_definition(&s, &observed.reference)
            .unwrap()
            .pointer(&observed.schema_pointers[0])
            .is_some()
    );
    let reference = KnowledgeEvidenceRef {
        kind: "observation".into(),
        id: id.to_string(),
    };
    assert!(snapshot_contains_reference(&s, &reference));
    let annotation = AnnotationDraft {
        id: Uuid::new_v4(),
        target: SemanticTarget::Field {
            field: formal.clone(),
        },
        value: SemanticValue::Description {
            text: "must remain unadopted".into(),
        },
        evidence: vec![KnowledgeEvidenceRef {
            kind: "field".into(),
            id: observed.id.clone(),
        }],
        verification: Verification::Inferred,
        note: "test".into(),
    };
    let plan = MaintenancePlan {
        strategy: MaintenanceStrategy::Keep,
        reason: "test".into(),
        expected_benefit: "test".into(),
        actions: vec![MaintenanceAction::UpsertAnnotation {
            annotation: Box::new(annotation),
            reason: "test".into(),
        }],
    };
    assert!(materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s)).is_err());
    let known: EvidenceFieldRef = formal.clone().into();
    assert_eq!(field_id(&formal), field_id(&known));
    s.inputs.as_mut().unwrap().observations[0].record.project_id = ProjectId::new();
    assert!(!snapshot_contains_reference(&s, &reference));
    s.inputs = None;
    let legacy_plan = MaintenancePlan {
        strategy: MaintenanceStrategy::Keep,
        reason: "old snapshot".into(),
        expected_benefit: "test".into(),
        actions: vec![],
    };
    assert!(materialize_maintenance(&s, &legacy_plan, Uuid::new_v4(), coverage(&s)).is_err());
}

#[test]
fn shared_review_transport_is_lossless_and_missing_context_is_rejected() {
    let mut s = snapshot();
    s.interfaces[0].environments[0].definition["response"]["body"]["observed_schema"] = json!({"type":"object","properties":{
        "empty":{"type":"null"},"quoted/key":{"type":"string"},"items":{"type":"array","items":{"type":"object","properties":{"id":{"type":"number"}}}}}});
    s.fields = complete_snapshot_fields(&s);
    let segments = review_segments(&s, 12000).unwrap();
    for segment in segments {
        let payload = segment.model_input();
        assert!(serde_json::to_vec(&payload).unwrap().len() <= 12000);
        let restored = ReviewSegment::from_model_input(payload.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(&segment).unwrap(),
            serde_json::to_value(restored).unwrap()
        );
        let mut bad = payload.clone();
        bad["units"][0]["context_ref"] = json!(99999);
        assert!(ReviewSegment::from_model_input(bad).is_err());
        let mut bad = payload;
        bad["ancestor_definitions"] = json!({});
        if segment.units.iter().any(|u| !u.ancestors.is_empty()) {
            assert!(ReviewSegment::from_model_input(bad).is_err());
        }
    }
}
#[test]
fn review_summary_budget_counts_json_escapes() {
    let s = snapshot();
    let segment = &review_segments(&s, 12000).unwrap()[0];
    let mut review = SegmentReview {
        segment_id: segment.id.clone(),
        summary: "必要发现".into(),
        assessments: segment
            .units
            .iter()
            .map(|u| FieldAssessment {
                unit_id: u.id.clone(),
                field_id: u.field_id.clone(),
                disposition: "keep".into(),
                note: "无新增证据".into(),
                evidence: vec![],
            })
            .collect(),
    };
    assert!(validate_segment(segment, &review).is_ok());
    review.summary = "\u{0001}".repeat(100);
    assert!(review.summary.len() < REVIEW_SUMMARY_BYTES);
    assert!(validate_segment(segment, &review).is_err());
}

#[test]
fn review_cannot_replace_relationship_evidence_with_only_a_field_reference() {
    let s = snapshot();
    let mut segment = review_segments(&s, 12000).unwrap().remove(0);
    let refs: Vec<_> = (0..2)
        .map(|_| KnowledgeEvidenceRef {
            kind: "fact".into(),
            id: Uuid::new_v4().to_string(),
        })
        .collect();
    segment.units[0].context["evidence_summary"] = json!({"representatives":[
        {"fact_kind":"parameter_link_candidate","reference":refs[0],"summary":{"ambiguous":true}},
        {"fact_kind":"parameter_link_counterexample","reference":refs[1],"summary":{"search_complete":false}}]});
    let mut review = SegmentReview {
        segment_id: segment.id.clone(),
        summary: "存在来源歧义，需回读".into(),
        assessments: segment
            .units
            .iter()
            .map(|u| FieldAssessment {
                unit_id: u.id.clone(),
                field_id: u.field_id.clone(),
                disposition: "needs_evidence".into(),
                note: "需要明确来源".into(),
                evidence: vec![KnowledgeEvidenceRef {
                    kind: "field".into(),
                    id: u.field_id.clone(),
                }],
            })
            .collect(),
    };
    assert!(validate_segment(&segment, &review).is_err());
    review.assessments[0].evidence.push(refs[0].clone());
    assert!(
        validate_segment(&segment, &review).is_err(),
        "counterexample also matters"
    );
    review.assessments[0].evidence.push(refs[1].clone());
    assert!(validate_segment(&segment, &review).is_ok());
    review.assessments[0].disposition = "keep".into();
    assert!(
        validate_segment(&segment, &review).is_ok(),
        "keeping existing knowledge is allowed with explicit evidence"
    );
}
#[test]
fn header_samples_are_not_enums_but_explicit_business_mappings_remain_possible() {
    let mut s = snapshot();
    s.interfaces[0].environments[0].definition["request"]["headers"][0]["name"] =
        json!("x-order-state");
    s.fields = complete_snapshot_fields(&s);
    let field = s
        .fields
        .iter()
        .find(|f| f.reference.path == "/x-order-state")
        .unwrap()
        .reference
        .adopted()
        .unwrap();
    let id = Uuid::new_v4();
    s.facts.push(EvidenceFact {
        id,
        project_id: s.project_id,
        environment_id: field.environment_id,
        kind: "observed_value".into(),
        subject: serde_json::to_value(&field).unwrap(),
        data: json!({"state":"present","value":"open"}),
        observations: 10,
        first_seen: "2026-09-16T00:00:00Z".into(),
        last_seen: "2026-09-16T00:00:01Z".into(),
        samples: vec![],
    });
    let mut plan = keep();
    plan.actions.push(MaintenanceAction::UpsertAnnotation {
        reason: "record observed status".into(),
        annotation: Box::new(AnnotationDraft {
            id: Uuid::new_v4(),
            target: SemanticTarget::Field {
                field: field.clone(),
            },
            value: SemanticValue::Enum {
                entries: vec![EnumEntry {
                    state: UiValueState::Present,
                    value: json!("open"),
                    label: None,
                }],
                complete: false,
                scope: json!({}),
            },
            evidence: vec![KnowledgeEvidenceRef {
                kind: "fact".into(),
                id: id.to_string(),
            }],
            verification: Verification::Observed,
            note: "sample only".into(),
        }),
    });
    assert!(
        matches!(materialize_maintenance(&s,&plan,Uuid::new_v4(),coverage(&s)),Err(Error::InvalidInput {message}) if message=="PROTOCOL_ENUM_REQUIRES_SEMANTIC_EVIDENCE")
    );
    s.facts[0].kind = "enum_label_candidate".into();
    s.facts[0].data["label"] = json!("进行中");
    assert!(materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s)).is_ok());
    s.facts[0].subject["path"] = json!("/another-header");
    assert!(materialize_maintenance(&s, &plan, Uuid::new_v4(), coverage(&s)).is_err());
}
