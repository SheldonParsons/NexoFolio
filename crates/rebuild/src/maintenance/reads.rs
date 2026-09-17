use super::*;
pub fn snapshot_contains_reference(snapshot: &KnowledgeSnapshot, r: &KnowledgeEvidenceRef) -> bool {
    match r.kind.as_str() {
        "field" => snapshot.fields.iter().any(|f| f.id == r.id),
        "interface" => snapshot
            .interfaces
            .iter()
            .any(|i| i.interface_id.to_string() == r.id),
        "fact" => snapshot.facts.iter().any(|f| f.id.to_string() == r.id),
        "directory" => snapshot
            .catalog
            .nodes
            .iter()
            .any(|n| n.id.to_string() == r.id),
        "image" => snapshot
            .facts
            .iter()
            .any(|f| f.kind == "page_image" && f.data["asset_id"].as_str() == Some(r.id.as_str())),
        "observation" => snapshot.inputs.as_ref().is_some_and(|i| {
            i.observations.iter().any(|m| {
                m.record.ingestion_id.to_string() == r.id
                    && m.record.project_id == snapshot.project_id
            })
        }),
        "source" => snapshot.inputs.as_ref().is_some_and(|i| {
            i.sources
                .iter()
                .any(|s| s.event_id.to_string() == r.id && s.project_id == snapshot.project_id)
        }),
        "summary" => false,
        _ => false,
    }
}
pub(super) fn refs_valid(snapshot: &KnowledgeSnapshot, refs: &[KnowledgeEvidenceRef]) -> bool {
    !refs.is_empty()
        && refs.iter().all(|r| {
            snapshot_contains_reference(snapshot, r)
                && (r.kind != "fact"
                    || snapshot
                        .facts
                        .iter()
                        .find(|f| f.id.to_string() == r.id)
                        .is_some_and(|f| f.data["needs_reassessment"] != true))
        })
}
pub(super) fn target_interface(target: &SemanticTarget) -> InterfaceId {
    match target {
        SemanticTarget::Interface { interface_id } => *interface_id,
        SemanticTarget::Field { field } => field.interface_id,
    }
}
pub fn required_reads(
    snapshot: &KnowledgeSnapshot,
    plan: &MaintenancePlan,
) -> Vec<KnowledgeEvidenceRef> {
    let mut set = HashSet::new();
    for action in &plan.actions {
        match action {
            MaintenanceAction::ReplaceCatalog { .. } => {
                for n in &snapshot.catalog.nodes {
                    set.insert(KnowledgeEvidenceRef {
                        kind: "directory".into(),
                        id: n.id.to_string(),
                    });
                }
            }
            MaintenanceAction::AssignInterface {
                directory_id: Some(d),
                ..
            } => {
                if snapshot.catalog.nodes.iter().any(|n| n.id == *d) {
                    set.insert(KnowledgeEvidenceRef {
                        kind: "directory".into(),
                        id: d.to_string(),
                    });
                }
            }
            MaintenanceAction::UpsertMergeGroup { group, .. } => {
                for i in &group.member_ids {
                    set.insert(KnowledgeEvidenceRef {
                        kind: "interface".into(),
                        id: i.to_string(),
                    });
                }
            }
            MaintenanceAction::RemoveMergeGroup {
                representative_id, ..
            } => {
                if let Some(g) = snapshot
                    .catalog
                    .merge_groups
                    .iter()
                    .find(|g| g.representative_id == *representative_id)
                {
                    for i in &g.member_ids {
                        set.insert(KnowledgeEvidenceRef {
                            kind: "interface".into(),
                            id: i.to_string(),
                        });
                    }
                }
            }
            _ => {}
        }
        let refs = match action {
            MaintenanceAction::UpsertAnnotation { annotation, .. } => {
                if snapshot
                    .annotations
                    .iter()
                    .any(|a| a.annotation.id == annotation.id)
                {
                    set.insert(KnowledgeEvidenceRef {
                        kind: "annotation".into(),
                        id: annotation.id.to_string(),
                    });
                }
                match &annotation.target {
                    SemanticTarget::Field { field } => {
                        set.insert(KnowledgeEvidenceRef {
                            kind: "field".into(),
                            id: field_id(field),
                        });
                    }
                    SemanticTarget::Interface { interface_id } => {
                        set.insert(KnowledgeEvidenceRef {
                            kind: "interface".into(),
                            id: interface_id.to_string(),
                        });
                    }
                }
                if let SemanticValue::ParameterRelation { source, target, .. } = &annotation.value {
                    for f in [source, target] {
                        set.insert(KnowledgeEvidenceRef {
                            kind: "field".into(),
                            id: field_id(f),
                        });
                    }
                }
                &annotation.evidence
            }
            MaintenanceAction::SetDirectory { node, evidence, .. } => {
                for id in std::iter::once(node.id).chain(node.parent) {
                    if snapshot.catalog.nodes.iter().any(|n| n.id == id) {
                        set.insert(KnowledgeEvidenceRef {
                            kind: "directory".into(),
                            id: id.to_string(),
                        });
                    }
                }
                evidence
            }
            MaintenanceAction::RemoveDirectory {
                directory_id,
                evidence,
                ..
            } => {
                set.insert(KnowledgeEvidenceRef {
                    kind: "directory".into(),
                    id: directory_id.to_string(),
                });
                evidence
            }
            MaintenanceAction::AssignInterface {
                interface_id,
                evidence,
                ..
            } => {
                set.insert(KnowledgeEvidenceRef {
                    kind: "interface".into(),
                    id: interface_id.to_string(),
                });
                evidence
            }
            MaintenanceAction::ReplaceCatalog { evidence, .. }
            | MaintenanceAction::UpsertMergeGroup { evidence, .. }
            | MaintenanceAction::RemoveMergeGroup { evidence, .. } => evidence,
            MaintenanceAction::RetractAnnotation {
                annotation_id,
                evidence,
                ..
            } => {
                set.insert(KnowledgeEvidenceRef {
                    kind: "annotation".into(),
                    id: annotation_id.to_string(),
                });
                evidence
            }
        };
        set.extend(refs.iter().cloned());
    }
    let mut refs: Vec<_> = set.into_iter().collect();
    refs.sort_by(|a, b| (&a.kind, &a.id).cmp(&(&b.kind, &b.id)));
    refs
}
