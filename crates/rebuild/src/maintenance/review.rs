use super::*;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewUnit {
    pub id: String,
    pub field_id: String,
    pub reference: FieldRef,
    pub ancestors: Vec<String>,
    pub context: Value,
    pub schema_entries: Vec<Value>,
    pub part: u32,
    pub parts: u32,
    pub annotations: Vec<SemanticAnnotation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSegment {
    pub id: String,
    pub units: Vec<ReviewUnit>,
}
fn atoms(value: &Value, path: &str, out: &mut Vec<Value>) {
    match value {
        Value::Object(o) if !o.is_empty() => {
            for (k, v) in o {
                atoms(v, &format!("{path}/{}", esc(k)), out)
            }
        }
        Value::Array(a) if !a.is_empty() => {
            for (i, v) in a.iter().enumerate() {
                atoms(v, &format!("{path}/{i}"), out)
            }
        }
        _ => out.push(json!({"pointer":path,"value":value})),
    }
}
pub fn review_segments(
    snapshot: &KnowledgeSnapshot,
    max_bytes: usize,
) -> Result<Vec<ReviewSegment>> {
    if max_bytes < 2048 {
        return Err(Error::invalid("model input budget too small"));
    }
    let mut units = Vec::new();
    let profiles = field_evidence_profiles(snapshot);
    let mut ordered: Vec<_> = snapshot.fields.iter().collect();
    ordered.sort_by_key(|f| {
        (
            f.reference.interface_id.to_string(),
            f.reference.environment_id.to_string(),
            f.reference.location.clone(),
            f.reference.path.clone(),
        )
    });
    for field in ordered {
        let mut context = review_field_context(snapshot, field);
        context["evidence_summary"] = profiles
            .get(&field.reference)
            .cloned()
            .unwrap_or_else(|| json!({"fact_count":0,"kinds":{},"representatives":[]}));
        context["stale_annotation_refs"]=json!(snapshot.annotations.iter().filter(|a|a.stale&&matches!(&a.annotation.target,SemanticTarget::Field{field:old} if old.interface_id==field.reference.interface_id&&old.environment_id==field.reference.environment_id&&old.location==field.reference.location&&old.path==field.reference.path)).map(|a|json!({"kind":"annotation","id":a.annotation.id})).collect::<Vec<_>>());
        let mut entries = Vec::new();
        atoms(&field.schema, "", &mut entries);
        let annotations: Vec<_> = snapshot
            .annotations
            .iter()
            .filter(|a| field.existing_annotations.contains(&a.annotation.id))
            .cloned()
            .collect();
        let make = |entries: Vec<Value>, part: u32| ReviewUnit {
            id: format!("{}:{part}", field.id),
            field_id: field.id.clone(),
            reference: field.reference.clone(),
            ancestors: field.ancestors.clone(),
            context: context.clone(),
            schema_entries: entries,
            part,
            parts: 0,
            annotations: annotations.clone(),
        };
        let mut fragments = Vec::new();
        let mut next = Vec::new();
        for entry in entries {
            let mut trial = next.clone();
            trial.push(entry.clone());
            if serde_json::to_vec(&make(trial.clone(), 0)).unwrap().len()
                > max_bytes.saturating_sub(512)
            {
                if next.is_empty() {
                    return Err(Error::invalid("FIELD_EXCEEDS_MODEL_BUDGET"));
                }
                fragments.push(make(std::mem::take(&mut next), fragments.len() as u32));
            }
            next.push(entry);
        }
        if !next.is_empty() {
            fragments.push(make(next, fragments.len() as u32));
        }
        let count = fragments.len() as u32;
        for mut part in fragments {
            part.parts = count;
            units.push(part);
        }
    }
    let mut result = Vec::new();
    let mut pending = Vec::new();
    let mut cursor = 0;
    while cursor < units.len() {
        let owner = units[cursor].reference.interface_id;
        let mut end = cursor + 1;
        while end < units.len() && units[end].reference.interface_id == owner {
            end += 1;
        }
        let group = &units[cursor..end];
        let mut trial = pending.clone();
        trial.extend_from_slice(group);
        if serde_json::to_vec(&trial).unwrap().len() <= max_bytes {
            pending = trial;
        } else {
            if !pending.is_empty() {
                result.push(ReviewSegment {
                    id: format!("segment-{}", result.len()),
                    units: std::mem::take(&mut pending),
                });
            }
            for unit in group {
                let mut trial = pending.clone();
                trial.push(unit.clone());
                if serde_json::to_vec(&trial).unwrap().len() > max_bytes {
                    if pending.is_empty() {
                        return Err(Error::invalid("REVIEW_UNIT_EXCEEDS_MODEL_BUDGET"));
                    }
                    result.push(ReviewSegment {
                        id: format!("segment-{}", result.len()),
                        units: std::mem::take(&mut pending),
                    });
                }
                pending.push(unit.clone());
            }
        }
        cursor = end;
    }
    if !pending.is_empty() {
        result.push(ReviewSegment {
            id: format!("segment-{}", result.len()),
            units: pending,
        });
    }
    Ok(result)
}
pub fn validate_segment(segment: &ReviewSegment, review: &SegmentReview) -> Result<()> {
    let expected: HashMap<_, _> = segment
        .units
        .iter()
        .map(|u| (u.id.as_str(), u.field_id.as_str()))
        .collect();
    let mut seen = HashSet::new();
    if segment.id != review.segment_id {
        return Err(Error::invalid("REVIEW_SEGMENT_MISMATCH"));
    }
    for a in &review.assessments {
        if expected.get(a.unit_id.as_str()) != Some(&a.field_id.as_str())
            || !seen.insert(a.unit_id.as_str())
            || a.note.trim().is_empty()
            || !["keep", "change", "conflict", "needs_evidence"].contains(&a.disposition.as_str())
        {
            return Err(Error::invalid("REVIEW_COVERAGE_INVALID"));
        }
    }
    if seen.len() != expected.len() {
        return Err(Error::invalid("REVIEW_COVERAGE_INCOMPLETE"));
    }
    Ok(())
}
