use super::*;
pub fn materialize_maintenance(
    snapshot: &KnowledgeSnapshot,
    plan: &MaintenancePlan,
    run: Uuid,
    coverage: ReviewCoverage,
) -> Result<MaintenanceCandidate> {
    let invalid = |s: &str| Error::InvalidInput { message: s.into() };
    if snapshot.inputs.is_none() {
        return Err(invalid("SNAPSHOT_INPUTS_UNAVAILABLE_RECREATE"));
    }
    let mut catalog = snapshot.catalog.clone();
    let mut annotations: HashMap<_, _> = snapshot
        .annotations
        .iter()
        .cloned()
        .map(|a| (a.annotation.id, a))
        .collect();
    if !coverage.complete || coverage.reviewed_fields != snapshot.fields.len() {
        return Err(invalid("MAINTENANCE_COVERAGE_INCOMPLETE"));
    }
    if plan.actions.len() > 10000 || plan.reason.trim().is_empty() {
        return Err(invalid("INVALID_MAINTENANCE_PLAN"));
    }
    let mut issues = Vec::new();
    for action in &plan.actions {
        match action {
            MaintenanceAction::SetDirectory {
                node,
                reason,
                evidence,
            } => {
                if reason.trim().is_empty()
                    || !refs_valid(snapshot, evidence)
                    || node.id == snapshot.system_directory_id
                    || node.parent == Some(snapshot.system_directory_id)
                    || node.name == "待分类"
                {
                    return Err(invalid("INVALID_DIRECTORY_CHANGE"));
                }
                if let Some(old) = catalog.nodes.iter_mut().find(|n| n.id == node.id) {
                    *old = node.clone();
                } else {
                    catalog.nodes.push(node.clone());
                }
            }
            MaintenanceAction::RemoveDirectory {
                directory_id,
                reason,
                evidence,
            } => {
                if reason.trim().is_empty()
                    || !refs_valid(snapshot, evidence)
                    || *directory_id == snapshot.system_directory_id
                    || !catalog.nodes.iter().any(|n| n.id == *directory_id)
                {
                    return Err(invalid("INVALID_DIRECTORY_REMOVAL"));
                }
                catalog.nodes.retain(|n| n.id != *directory_id);
            }
            MaintenanceAction::AssignInterface {
                interface_id,
                directory_id,
                reason,
                evidence,
            } => {
                if reason.trim().is_empty() || !refs_valid(snapshot, evidence) {
                    return Err(invalid("INVALID_ASSIGNMENT_BASIS"));
                }
                let a = catalog
                    .assignments
                    .iter_mut()
                    .find(|a| a.interface_id == *interface_id)
                    .ok_or_else(|| invalid("UNKNOWN_INTERFACE"))?;
                a.directory_id = *directory_id;
                a.reason = reason.clone();
            }
            MaintenanceAction::ReplaceCatalog {
                candidate,
                reason,
                evidence,
            } => {
                if !matches!(plan.strategy, MaintenanceStrategy::Full)
                    || reason.trim().is_empty()
                    || !refs_valid(snapshot, evidence)
                {
                    return Err(invalid("INVALID_FULL_REBUILD"));
                }
                catalog = candidate.clone();
            }
            MaintenanceAction::UpsertMergeGroup {
                group,
                reason,
                evidence,
            } => {
                if reason.trim().is_empty() || !refs_valid(snapshot, evidence) {
                    return Err(invalid("INVALID_MERGE_BASIS"));
                }
                catalog
                    .merge_groups
                    .retain(|g| g.representative_id != group.representative_id);
                catalog.merge_groups.push(group.clone());
            }
            MaintenanceAction::RemoveMergeGroup {
                representative_id,
                reason,
                evidence,
            } => {
                if reason.trim().is_empty() || !refs_valid(snapshot, evidence) {
                    return Err(invalid("INVALID_MERGE_REMOVAL"));
                }
                catalog
                    .merge_groups
                    .retain(|g| g.representative_id != *representative_id);
            }
            MaintenanceAction::UpsertAnnotation { annotation, reason } => {
                if reason.trim().is_empty() || !refs_valid(snapshot, &annotation.evidence) {
                    return Err(invalid("ANNOTATION_REQUIRES_EVIDENCE"));
                }
                if !annotation.evidence.iter().any(|r| {
                    r.kind == "field"
                        || r.kind == "fact"
                        || (r.kind == "interface"
                            && matches!(annotation.value, SemanticValue::Description { .. })
                            && r.id == target_interface(&annotation.target).to_string())
                }) {
                    return Err(invalid("ANNOTATION_REQUIRES_FIELD_OR_FACT"));
                }
                let owner = target_interface(&annotation.target);
                let interface = snapshot
                    .interfaces
                    .iter()
                    .find(|i| i.interface_id == owner)
                    .ok_or_else(|| invalid("UNKNOWN_ANNOTATION_INTERFACE"))?;
                if let SemanticTarget::Field { field } = &annotation.target
                    && !snapshot.fields.iter().any(|f| f.reference == *field)
                {
                    return Err(invalid("UNKNOWN_ANNOTATION_FIELD"));
                }
                match &annotation.value {
                    SemanticValue::Description { text } => {
                        if text.trim().is_empty() || text.chars().count() > 4000 {
                            return Err(invalid("INVALID_DESCRIPTION"));
                        }
                    }
                    SemanticValue::ParameterRelation {
                        source,
                        target,
                        transform,
                        ..
                    } => {
                        if !matches!(&annotation.target,SemanticTarget::Field{field} if field==target)
                            || !source.location.starts_with("response.")
                            || !target.location.starts_with("request.")
                        {
                            return Err(invalid("INVALID_RELATION_DIRECTION"));
                        }
                        if source.environment_id != target.environment_id
                            || source.interface_id == target.interface_id
                            || !["identity", "number_to_string", "string_to_number"]
                                .contains(&transform.as_str())
                            || ![source, target]
                                .iter()
                                .all(|f| snapshot.fields.iter().any(|x| x.reference == **f))
                        {
                            return Err(invalid("INVALID_PARAMETER_RELATION"));
                        }
                        let support = snapshot
                            .facts
                            .iter()
                            .filter(|f| {
                                annotation
                                    .evidence
                                    .iter()
                                    .any(|r| r.kind == "fact" && r.id == f.id.to_string())
                            })
                            .find(|f| {
                                f.kind == "parameter_link_candidate"
                                    && f.subject["source"] == serde_json::to_value(source).unwrap()
                                    && f.subject["target"] == serde_json::to_value(target).unwrap()
                                    && f.data["transform"] == *transform
                            });
                        let Some(support) = support else {
                            return Err(invalid("RELATION_REQUIRES_CAPTURE_SUPPORT"));
                        };
                        if (support.data["ambiguous"] == true || support.data["conflict"] == true)
                            && !matches!(annotation.verification, Verification::NeedsReview)
                        {
                            return Err(invalid("RELATION_UNCERTAINTY_MUST_REMAIN_VISIBLE"));
                        }
                        if matches!(annotation.verification, Verification::Observed) {
                            return Err(invalid("CORRELATION_IS_NOT_CAUSAL_PROOF"));
                        }
                    }
                    SemanticValue::Enum {
                        entries, complete, ..
                    } => {
                        if !matches!(annotation.target, SemanticTarget::Field { .. }) {
                            return Err(invalid("ENUM_REQUIRES_FIELD"));
                        }

                        if entries.len() > 500 {
                            return Err(invalid("ENUM_LIMIT"));
                        }
                        if *complete
                            && !annotation.evidence.iter().any(|r| {
                                r.kind == "fact"
                                    && snapshot.facts.iter().any(|f| {
                                        f.id.to_string() == r.id
                                            && f.kind == "declared_enum"
                                            && f.data["complete_enum"] == true
                                    })
                            })
                        {
                            return Err(invalid("ENUM_COMPLETENESS_UNSUPPORTED"));
                        }
                        let SemanticTarget::Field { field } = &annotation.target else {
                            unreachable!()
                        };
                        let subject = serde_json::to_value(field).unwrap();
                        let evidence: Vec<_> = snapshot
                            .facts
                            .iter()
                            .filter(|f| {
                                annotation
                                    .evidence
                                    .iter()
                                    .any(|r| r.kind == "fact" && r.id == f.id.to_string())
                            })
                            .collect();
                        if field.location.ends_with(".header")
                            && !evidence.iter().any(|f| {
                                f.subject == subject
                                    && matches!(
                                        f.kind.as_str(),
                                        "declared_enum"
                                            | "dictionary_mapping_candidate"
                                            | "enum_label_candidate"
                                    )
                            })
                        {
                            return Err(invalid("PROTOCOL_ENUM_REQUIRES_SEMANTIC_EVIDENCE"));
                        }
                        if let Some(old) = annotations.get(&annotation.id)
                            && let SemanticValue::Enum { entries: prior, .. } =
                                &old.annotation.value
                        {
                            for removed in prior.iter().filter(|old| {
                                !entries.iter().any(|new| {
                                    serde_json::to_value(&new.state).unwrap()
                                        == serde_json::to_value(&old.state).unwrap()
                                        && new.value == old.value
                                })
                            }) {
                                if !evidence.iter().any(|f| {
                                    f.kind == "enum_value_invalidated"
                                        && f.subject == subject
                                        && f.data["counterexample"] == true
                                        && f.data["state"]
                                            == serde_json::to_value(&removed.state).unwrap()
                                        && f.data["value"] == removed.value
                                }) {
                                    return Err(invalid("ENUM_REMOVAL_REQUIRES_COUNTEREVIDENCE"));
                                }
                            }
                        }
                        let mut values = HashSet::new();
                        for entry in entries {
                            let state = serde_json::to_value(&entry.state).unwrap();
                            if !evidence.iter().any(|f| {
                                f.subject == subject
                                    && f.data["state"] == state
                                    && f.data["value"] == entry.value
                            }) {
                                return Err(invalid("ENUM_VALUE_REQUIRES_FIELD_EVIDENCE"));
                            }
                            if entry.label.is_some()
                                && matches!(annotation.verification, Verification::Observed)
                                && !evidence.iter().any(|f| {
                                    f.subject == subject
                                        && f.data["state"] == state
                                        && f.data["value"] == entry.value
                                        && f.data["label"].as_str() == entry.label.as_deref()
                                })
                            {
                                return Err(invalid("ENUM_MAPPING_IS_INFERRED"));
                            }
                            if let Some(label) = &entry.label
                                && !evidence.iter().any(|f| {
                                    f.data["state"] == state
                                        && f.data["value"] == entry.value
                                        && f.data["label"] == *label
                                })
                            {
                                return Err(invalid("ENUM_LABEL_REQUIRES_OBSERVED_MAPPING"));
                            }
                            if evidence.iter().any(|f| f.data["conflict"] == true)
                                && !matches!(annotation.verification, Verification::NeedsReview)
                            {
                                return Err(invalid("ENUM_CONFLICT_MUST_REMAIN_VISIBLE"));
                            }

                            if !matches!(entry.state, UiValueState::Present)
                                && !entry.value.is_null()
                            {
                                return Err(invalid("INVALID_ABSENT_ENUM_VALUE"));
                            }
                            if !values.insert(
                                serde_json::to_string(&(
                                    entry.state.clone(),
                                    entry.value.clone(),
                                    entry.label.clone(),
                                ))
                                .unwrap(),
                            ) {
                                return Err(invalid("DUPLICATE_ENUM_ENTRY"));
                            }
                        }
                    }
                }
                let mut basis = Vec::new();
                match &annotation.target {
                    SemanticTarget::Interface { .. } => {
                        basis.extend(interface.environments.iter().map(|e| DefinitionBasis {
                            interface_id: owner,
                            environment_id: e.environment_id,
                            revision_id: e.revision_id,
                        }))
                    }
                    SemanticTarget::Field { field } => basis.push(DefinitionBasis {
                        interface_id: field.interface_id,
                        environment_id: field.environment_id,
                        revision_id: field.revision_id,
                    }),
                }
                for r in &annotation.evidence {
                    if r.kind == "field"
                        && let Some(f) = snapshot.fields.iter().find(|f| f.id == r.id)
                        && let Some(reference) = f.reference.adopted()
                    {
                        basis.push(DefinitionBasis {
                            interface_id: reference.interface_id,
                            environment_id: reference.environment_id,
                            revision_id: reference.revision_id,
                        });
                    }
                }
                if let SemanticValue::ParameterRelation { source, target, .. } = &annotation.value {
                    for f in [source, target] {
                        basis.push(DefinitionBasis {
                            interface_id: f.interface_id,
                            environment_id: f.environment_id,
                            revision_id: f.revision_id,
                        });
                    }
                }
                basis.sort_by_key(|b| {
                    (
                        b.interface_id.to_string(),
                        b.environment_id.to_string(),
                        b.revision_id.to_string(),
                    )
                });
                basis.dedup_by(|a, b| {
                    a.interface_id == b.interface_id
                        && a.environment_id == b.environment_id
                        && a.revision_id == b.revision_id
                });
                annotations.insert(
                    annotation.id,
                    SemanticAnnotation {
                        annotation: annotation.as_ref().clone(),
                        basis,
                        source_run_id: run,
                        stale: false,
                    },
                );
            }
            MaintenanceAction::RetractAnnotation {
                annotation_id,
                reason,
                evidence,
            } => {
                if let Some(old) = annotations.get(annotation_id)
                    && let SemanticValue::Enum {
                        entries, complete, ..
                    } = &old.annotation.value
                {
                    let target = match &old.annotation.target {
                        SemanticTarget::Field { field } => serde_json::to_value(field).unwrap(),
                        _ => Value::Null,
                    };
                    let counter = snapshot.facts.iter().any(|f| {
                        evidence
                            .iter()
                            .any(|r| r.kind == "fact" && r.id == f.id.to_string())
                            && f.subject == target
                            && (f.data["conflict"] == true
                                || f.data["counterexample"] == true
                                || (*complete
                                    && f.kind == "observed_value"
                                    && !entries.iter().any(|e| {
                                        e.value == f.data["value"]
                                            && serde_json::to_value(&e.state).unwrap()
                                                == f.data["state"]
                                    })))
                    });
                    if !counter {
                        return Err(invalid("ENUM_RETRACTION_REQUIRES_COUNTEREVIDENCE"));
                    }
                }
                if reason.trim().is_empty()
                    || !refs_valid(snapshot, evidence)
                    || annotations.remove(annotation_id).is_none()
                {
                    return Err(invalid("INVALID_ANNOTATION_RETRACTION"));
                }
            }
        }
    }
    if matches!(plan.strategy, MaintenanceStrategy::Insert) {
        let preserves_nodes = snapshot.catalog.nodes.iter().all(|old| {
            catalog.nodes.iter().any(|new| {
                new.id == old.id
                    && new.parent == old.parent
                    && new.name == old.name
                    && new.description == old.description
            })
        });
        let preserves_assignments = snapshot
            .catalog
            .assignments
            .iter()
            .filter(|a| a.directory_id.is_some())
            .all(|old| {
                catalog.assignments.iter().any(|new| {
                    new.interface_id == old.interface_id && new.directory_id == old.directory_id
                })
            });
        let preserves_groups = snapshot.catalog.merge_groups.iter().all(|old| {
            catalog
                .merge_groups
                .iter()
                .any(|new| serde_json::to_value(new).unwrap() == serde_json::to_value(old).unwrap())
        });
        if !preserves_nodes || !preserves_assignments || !preserves_groups {
            return Err(invalid("INSERT_STRATEGY_CHANGED_EXISTING_CATALOG"));
        }
    }
    if matches!(plan.strategy, MaintenanceStrategy::Keep)
        && serde_json::to_value(&catalog).unwrap()
            != serde_json::to_value(&snapshot.catalog).unwrap()
    {
        return Err(invalid("KEEP_STRATEGY_CHANGED_DIRECTORY"));
    }
    if catalog
        .nodes
        .iter()
        .any(|n| n.id == snapshot.system_directory_id || n.name == "待分类")
    {
        return Err(invalid("SYSTEM_DIRECTORY_IMMUTABLE"));
    }
    let legacy = CatalogSnapshot {
        project_id: snapshot.project_id,
        project_name: "maintenance".into(),
        interfaces: snapshot.interfaces.clone(),
    };
    let review =
        crate::DirectoryReviewer::review(&crate::StructuralDirectoryReviewer, &legacy, &catalog);
    if !review.structurally_valid {
        issues.extend(review.issues.iter().map(|i| i.code.clone()));
    }
    let mut unique = HashSet::new();
    for a in annotations.values() {
        let kind = match a.annotation.value {
            SemanticValue::Description { .. } => "description",
            SemanticValue::ParameterRelation { .. } => "relation",
            SemanticValue::Enum { .. } => "enum",
        };
        let identity = serde_json::to_string(&(kind, &a.annotation.target)).unwrap();
        if !unique.insert(identity) {
            issues.push("DUPLICATE_ACTIVE_ANNOTATION_TARGET".into());
        }
    }
    let mut annotations: Vec<_> = annotations.into_values().collect();
    annotations.sort_by_key(|a| a.annotation.id);
    Ok(MaintenanceCandidate {
        plan: plan.clone(),
        catalog,
        annotations,
        review,
        coverage,
        issues,
    })
}
