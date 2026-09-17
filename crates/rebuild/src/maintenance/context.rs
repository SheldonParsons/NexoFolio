use super::*;
pub(super) fn field_evidence_profiles(
    snapshot: &KnowledgeSnapshot,
) -> HashMap<EvidenceFieldRef, Value> {
    let mut grouped: HashMap<EvidenceFieldRef, BTreeMap<String, Vec<&EvidenceFact>>> =
        HashMap::new();
    for fact in &snapshot.facts {
        let refs = evidence_field_refs(&fact.subject);
        for reference in refs {
            grouped
                .entry(reference)
                .or_default()
                .entry(fact.kind.clone())
                .or_default()
                .push(fact);
        }
    }
    grouped.into_iter().map(|(reference,kinds)|{
  let mut counts=BTreeMap::new();let mut representatives=vec![];let mut total=0;
  for (kind,mut facts) in kinds {counts.insert(kind.clone(),facts.len());total+=facts.len();facts.sort_by(|a,b|{let uncertain=|f:&EvidenceFact|f.data["conflict"]==true||f.data["counterexample"]==true;(uncertain(b),&b.last_seen,&b.id).cmp(&(uncertain(a),&a.last_seen,&a.id))});
   for fact in facts.into_iter().take(3){let mut omitted=vec![];let mut detail=json!({"conflict":fact.data["conflict"],"ambiguous":fact.data["ambiguous"],"distinct_values_lower_bound":fact.data["distinct_values_lower_bound"]});for key in ["value","label","state","verification","scope","transform","search_complete","support_kind","evidence_rule_version","needs_reassessment","relation_exclusion","retained_value_limit","complete_enum","reason","policy_version"]{if let Some(v)=fact.data.get(key){if serde_json::to_vec(v).unwrap().len()<=160{detail[key]=v.clone();}else{omitted.push(key);}}}detail["omitted_fields"]=json!(omitted);representatives.push(json!({"reference":{"kind":"fact","id":fact.id},"fact_kind":kind,"summary":detail}));}
  }
  (reference,json!({"fact_count":total,"kinds":counts,"sampled":representatives.len()<total,"representatives":representatives,"meaning":"Evidence navigation, not complete enum constraints. Read referenced originals before changing knowledge."}))
 }).collect()
}

/// Structured, lossless field identities paired with bounded evidence navigation summaries.
pub fn field_maintenance_hints(snapshot: &KnowledgeSnapshot) -> Vec<Value> {
    let known: HashSet<_> = snapshot
        .fields
        .iter()
        .map(|f| f.reference.clone())
        .collect();
    let mut hints = vec![];
    for (reference, profile) in field_evidence_profiles(snapshot) {
        if !known.contains(&reference) {
            continue;
        }
        if [
            "parameter_link_candidate",
            "parameter_link_counterexample",
            "enum_label_candidate",
            "dictionary_mapping_candidate",
            "ui_field_label",
        ]
        .iter()
        .any(|kind| profile["kinds"].get(kind).is_some())
        {
            hints.push(
                json!({"field_id":field_id(&reference),"field_ref":reference,"evidence":profile}),
            );
        }
    }
    hints.sort_by_key(|v| v["field_id"].as_str().unwrap().to_owned());
    hints
}

/// Distinguish absent schema from a JSON null sample and retain structural ancestors.
pub fn review_field_context(snapshot: &KnowledgeSnapshot, field: &KnowledgeField) -> Value {
    let interface = snapshot
        .interfaces
        .iter()
        .find(|i| i.interface_id == field.reference.interface_id);
    let environment = interface.and_then(|i| {
        i.environments
            .iter()
            .find(|e| e.environment_id == field.reference.environment_id)
    });
    let material = field.reference.observation.as_ref().and_then(|r| {
        snapshot
            .inputs
            .as_ref()?
            .observations
            .iter()
            .find(|m| m.record.ingestion_id == r.ingestion_id)
    });
    let side = field.reference.location.split('.').next().unwrap_or("");
    let body = field_definition(snapshot, &field.reference)
        .map(|d| {
            let mut value = d[side]["body"].clone();
            if let Some(o) = value.as_object_mut() {
                o.remove("observed_schema");
            }
            value
        })
        .unwrap_or(Value::Null);
    let ancestors: Vec<_> = field
        .ancestors
        .iter()
        .filter_map(|id| snapshot.fields.iter().find(|f| f.id == *id))
        .map(|f| {
            fn parent_outline(v: &Value) -> Value {
                match v {
                    Value::Object(o) => Value::Object(
                        o.iter()
                            .filter(|(k, _)| !matches!(k.as_str(), "properties" | "items"))
                            .map(|(k, v)| (k.clone(), parent_outline(v)))
                            .collect(),
                    ),
                    Value::Array(a) => Value::Array(a.iter().map(parent_outline).collect()),
                    _ => v.clone(),
                }
            }
            let schema = parent_outline(&f.schema);
            json!({"id":f.id,"reference":f.reference,"schema":schema})
        })
        .collect();
    json!({"method":interface.map(|i|&i.method),"path":interface.map(|i|&i.path),"environment_name":environment.map(|e|&e.environment_name),"body_capture":body,"schema_view":field_definition(snapshot,&field.reference).map(|d|&d["schema_view"]),"schema_not_observed":field.schema.is_null(),"definition_origin":if field.reference.adopted().is_some(){"adopted_revision"}else if material.is_some_and(|m|m.reconstructed_definition.is_some()){"raw_reextracted"}else{"stored_observation"},"assessment_categories":material.and_then(|m|m.record.assessment.as_ref()).map(|a|&a.categories),"writable":field.reference.adopted().is_some(),"observation_source":field.reference.observation,"definition_limitations":field_definition(snapshot,&field.reference).map(|d|&d["limitations"]),"ancestor_structure":ancestors})
}
