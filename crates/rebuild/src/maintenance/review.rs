use super::*;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewUnit {
    pub id: String,
    pub field_id: String,
    pub reference: EvidenceFieldRef,
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
impl ReviewSegment {
    /// Transport-only normalization. Original units remain the coverage authority.
    pub fn model_input(&self) -> Value {
        let mut contexts = Vec::<Value>::new();
        let mut ancestors = BTreeMap::new();
        let mut references = Vec::<Value>::new();
        let units: Vec<_> = self
            .units
            .iter()
            .map(|unit| {
                let mut value = serde_json::to_value(unit).expect("unit");
                let mut context = value["context"].take();
                for mut ancestor in context
                    .as_object_mut()
                    .unwrap()
                    .remove("ancestor_structure")
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default()
                {
                    ancestor["reference"] =
                        pack_reference(ancestor["reference"].take(), &mut references);
                    ancestors.insert(ancestor["id"].as_str().unwrap().to_owned(), ancestor);
                }
                let mut details = serde_json::Map::new();
                for name in [
                    "schema_not_observed",
                    "evidence_summary",
                    "stale_annotation_refs",
                ] {
                    if let Some(v) = context.as_object_mut().unwrap().remove(name) {
                        details.insert(name.into(), v);
                    }
                }
                let reference = value.as_object_mut().unwrap().remove("reference").unwrap();
                let packed_reference = pack_reference(reference, &mut references);
                value["path"] = packed_reference["path"].clone();
                if let Some(source) = context["observation_source"].as_object_mut()
                    && source.get("path") == Some(&value["path"])
                {
                    source.remove("path");
                }
                let shared =
                    json!({"reference_ref":packed_reference["reference_ref"],"context":context});
                let index = contexts
                    .iter()
                    .position(|v| v == &shared)
                    .unwrap_or_else(|| {
                        contexts.push(shared);
                        contexts.len() - 1
                    });
                value["context_ref"] = json!(index);
                value["context"] = Value::Object(details);
                value
            })
            .collect();
        json!({"id":self.id,"contexts":contexts,"references":references,"ancestor_definitions":ancestors,"units":units})
    }

    /// Lossless inverse, also used when validating replies against the transmitted manifest.
    pub fn from_model_input(mut input: Value) -> Result<Self> {
        let contexts = input
            .get("contexts")
            .cloned()
            .ok_or_else(|| Error::invalid("REVIEW_CONTEXT_MISSING"))?;
        let references = input
            .get("references")
            .cloned()
            .ok_or_else(|| Error::invalid("REVIEW_REFERENCE_MISSING"))?;
        let mut ancestors = input
            .get("ancestor_definitions")
            .cloned()
            .ok_or_else(|| Error::invalid("REVIEW_ANCESTOR_MISSING"))?;
        for ancestor in ancestors
            .as_object_mut()
            .ok_or_else(|| Error::invalid("REVIEW_ANCESTOR_MISSING"))?
            .values_mut()
        {
            ancestor["reference"] = unpack_reference(&ancestor["reference"], &references)?;
        }
        let units = input["units"]
            .as_array_mut()
            .ok_or_else(|| Error::invalid("REVIEW_UNITS_MISSING"))?;
        for unit in units {
            let index = unit["context_ref"]
                .as_u64()
                .ok_or_else(|| Error::invalid("REVIEW_CONTEXT_MISSING"))?
                as usize;
            let shared = contexts
                .get(index)
                .ok_or_else(|| Error::invalid("REVIEW_CONTEXT_MISSING"))?;
            let mut context = shared["context"]
                .as_object()
                .cloned()
                .ok_or_else(|| Error::invalid("REVIEW_CONTEXT_MISSING"))?;
            context.extend(
                unit["context"]
                    .as_object()
                    .cloned()
                    .ok_or_else(|| Error::invalid("REVIEW_CONTEXT_MISSING"))?,
            );
            let parents = unit["ancestors"]
                .as_array()
                .ok_or_else(|| Error::invalid("REVIEW_ANCESTOR_MISSING"))?
                .iter()
                .map(|id| {
                    id.as_str()
                        .and_then(|id| ancestors.get(id))
                        .cloned()
                        .ok_or_else(|| Error::invalid("REVIEW_ANCESTOR_MISSING"))
                })
                .collect::<Result<Vec<_>>>()?;
            context.insert("ancestor_structure".into(), json!(parents));
            unit["context"] = Value::Object(context);
            unit["reference"] = unpack_reference(
                &json!({"reference_ref":shared["reference_ref"],"path":unit["path"]}),
                &references,
            )?;
            let path = unit["path"].clone();
            if let Some(source) = unit["context"]["observation_source"].as_object_mut() {
                source.entry("path").or_insert(path);
            }
        }
        serde_json::from_value(input).map_err(|_| Error::invalid("REVIEW_INPUT_INVALID"))
    }
}
fn pack_reference(mut reference: Value, references: &mut Vec<Value>) -> Value {
    let path = reference
        .as_object_mut()
        .expect("typed field reference")
        .remove("path")
        .unwrap();
    if let Some(observation) = reference
        .get_mut("observation")
        .and_then(Value::as_object_mut)
        && observation.get("path") == Some(&path)
    {
        observation.remove("path");
    }
    let index = references
        .iter()
        .position(|v| v == &reference)
        .unwrap_or_else(|| {
            references.push(reference);
            references.len() - 1
        });
    json!({"reference_ref":index,"path":path})
}
fn unpack_reference(packed: &Value, references: &Value) -> Result<Value> {
    let mut reference = packed["reference_ref"]
        .as_u64()
        .and_then(|n| references.get(n as usize))
        .cloned()
        .ok_or_else(|| Error::invalid("REVIEW_REFERENCE_MISSING"))?;
    let path = packed
        .get("path")
        .cloned()
        .ok_or_else(|| Error::invalid("REVIEW_PATH_MISSING"))?;
    reference["path"] = path.clone();
    if let Some(observation) = reference
        .get_mut("observation")
        .and_then(Value::as_object_mut)
    {
        observation.entry("path").or_insert(path);
    }
    Ok(reference)
}
fn packed_bytes(units: &[ReviewUnit]) -> usize {
    serde_json::to_vec(
        &ReviewSegment {
            id: "segment-999999".into(),
            units: units.to_vec(),
        }
        .model_input(),
    )
    .unwrap()
    .len()
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
        // Entries are independent JSON atoms; count each once rather than serializing
        // the growing field and its repeated context for every atom (quadratic work).
        let overhead = packed_bytes(&[make(Vec::new(), 0)]) + 32;
        let mut size = overhead;
        for entry in entries {
            let entry_bytes = serde_json::to_vec(&entry).unwrap().len() + 1;
            if size + entry_bytes > max_bytes {
                if next.is_empty() {
                    return Err(Error::invalid("FIELD_EXCEEDS_MODEL_BUDGET"));
                }
                fragments.push(make(std::mem::take(&mut next), fragments.len() as u32));
                size = overhead;
            }
            if size + entry_bytes > max_bytes {
                return Err(Error::invalid("FIELD_EXCEEDS_MODEL_BUDGET"));
            }
            size += entry_bytes;
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
        if packed_bytes(&trial) <= max_bytes && trial.len() <= 48 {
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
                if packed_bytes(&trial) > max_bytes || trial.len() > 48 {
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
pub const REVIEW_SUMMARY_BYTES: usize = 512;
pub fn validate_segment(segment: &ReviewSegment, review: &SegmentReview) -> Result<()> {
    if review.summary.trim().is_empty()
        || serde_json::to_vec(&review.summary).unwrap().len() > REVIEW_SUMMARY_BYTES
    {
        return Err(Error::invalid("REVIEW_SUMMARY_BUDGET_EXCEEDED"));
    }
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
    for unit in &segment.units {
        let assessment = review
            .assessments
            .iter()
            .find(|a| a.unit_id == unit.id)
            .ok_or_else(|| Error::invalid("REVIEW_COVERAGE_INCOMPLETE"))?;
        for representative in unit.context["evidence_summary"]["representatives"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if matches!(
                representative["fact_kind"].as_str(),
                Some("parameter_link_candidate" | "parameter_link_counterexample")
            ) {
                let reference: KnowledgeEvidenceRef =
                    serde_json::from_value(representative["reference"].clone())
                        .map_err(|_| Error::invalid("REVIEW_RELATION_REFERENCE_INVALID"))?;
                if !assessment.evidence.contains(&reference) {
                    return Err(Error::invalid("REVIEW_RELATION_EVIDENCE_UNACKNOWLEDGED"));
                }
            }
        }
    }
    if seen.len() != expected.len() {
        return Err(Error::invalid("REVIEW_COVERAGE_INCOMPLETE"));
    }
    Ok(())
}
