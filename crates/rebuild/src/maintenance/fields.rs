use super::*;
pub(super) fn key(value: &Value) -> String {
    Sha256::digest(serde_json::to_vec(value).expect("serializes"))
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn field_id(reference: &impl Serialize) -> String {
    format!(
        "f_{}",
        &key(&serde_json::to_value(reference).unwrap())[..32]
    )
}
pub(super) fn esc(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}
fn child_ref(base: &EvidenceFieldRef, path: String) -> EvidenceFieldRef {
    base.at(base.location.clone(), path)
}
fn local_schema(schema: &Value, reference: &EvidenceFieldRef) -> Value {
    let Some(map) = schema.as_object() else {
        return schema.clone();
    };
    let mut out = map.clone();
    if let Some(props) = map.get("properties").and_then(Value::as_object) {
        out.insert(
            "properties".into(),
            Value::Object(
                props
                    .keys()
                    .map(|name| {
                        let r = child_ref(reference, format!("{}/{}", reference.path, esc(name)));
                        (name.clone(), json!({"field_id":field_id(&r)}))
                    })
                    .collect(),
            ),
        );
    }
    if map.contains_key("items") {
        out.insert(
            "items".into(),
            json!({"field_id":field_id(&child_ref(reference,format!("{}/*",reference.path)))}),
        );
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(options) = map.get(key).and_then(Value::as_array) {
            out.insert(
                key.into(),
                Value::Array(options.iter().map(|v| local_schema(v, reference)).collect()),
            );
        }
    }
    Value::Object(out)
}
fn collect_schema(
    schema: &Value,
    reference: EvidenceFieldRef,
    pointer: String,
    ancestors: Vec<String>,
    out: &mut BTreeMap<String, KnowledgeField>,
) {
    let id = field_id(&reference);
    let local = local_schema(schema, &reference);
    let entry = out.entry(id.clone()).or_insert_with(|| KnowledgeField {
        id: id.clone(),
        reference: reference.clone(),
        schema: local.clone(),
        schema_pointers: vec![],
        ancestors: ancestors.clone(),
        existing_annotations: vec![],
    });
    if entry.reference != reference {
        panic!("field identity collision");
    }
    if entry.schema != local
        && !entry
            .schema
            .get("anyOf")
            .and_then(Value::as_array)
            .is_some_and(|a| a.contains(&local))
    {
        entry.schema = json!({"anyOf":[entry.schema.clone(),local]});
    }
    if !entry.schema_pointers.contains(&pointer) {
        entry.schema_pointers.push(pointer.clone());
    }
    let mut parents = ancestors;
    parents.push(id);
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        for (name, child) in props {
            collect_schema(
                child,
                child_ref(&reference, format!("{}/{}", reference.path, esc(name))),
                format!("{pointer}/properties/{}", esc(name)),
                parents.clone(),
                out,
            );
        }
    }
    if let Some(items) = schema.get("items") {
        collect_schema(
            items,
            child_ref(&reference, format!("{}/*", reference.path)),
            format!("{pointer}/items"),
            parents.clone(),
            out,
        );
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(options) = schema.get(key).and_then(Value::as_array) {
            for (i, child) in options.iter().enumerate() {
                collect_schema(
                    child,
                    reference.clone(),
                    format!("{pointer}/{key}/{i}"),
                    parents[..parents.len() - 1].to_vec(),
                    out,
                );
            }
        }
    }
}
fn definition_fields(
    base: EvidenceFieldRef,
    d: &Value,
    fields: &mut BTreeMap<String, KnowledgeField>,
) {
    for side in ["request", "response"] {
        collect_schema(
            &d[side]["body"]["observed_schema"],
            base.at(format!("{side}.body"), String::new()),
            format!("/{side}/body/observed_schema"),
            vec![],
            fields,
        );
        for (index, header) in d[side]["headers"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if let Some(name) = header["name"].as_str() {
                collect_schema(
                    &header["observed_schema"],
                    base.at(format!("{side}.header"), format!("/{}", esc(name))),
                    format!("/{side}/headers/{index}/observed_schema"),
                    vec![],
                    fields,
                );
            }
        }
    }
    for (index, p) in d["request"]["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if let (Some(name), Some(location)) = (p["name"].as_str(), p["in"].as_str()) {
            collect_schema(
                &p["observed_schema"],
                base.at(format!("request.{location}"), format!("/{}", esc(name))),
                format!("/request/parameters/{index}/observed_schema"),
                vec![],
                fields,
            );
        }
    }
}
pub fn snapshot_fields(
    interfaces: &[CatalogInterface],
    annotations: &[SemanticAnnotation],
) -> Vec<KnowledgeField> {
    let mut fields = BTreeMap::new();
    for i in interfaces {
        for e in &i.environments {
            definition_fields(
                FieldRef {
                    interface_id: i.interface_id,
                    environment_id: e.environment_id,
                    revision_id: e.revision_id,
                    location: String::new(),
                    path: String::new(),
                }
                .into(),
                &e.definition,
                &mut fields,
            );
        }
    }
    for annotation in annotations {
        if let SemanticTarget::Field { field } = &annotation.annotation.target
            && let Some(f) = fields.get_mut(&field_id(field))
        {
            f.existing_annotations.push(annotation.annotation.id);
        }
    }
    fields.into_values().collect()
}
/// One field parser for profile navigation, unresolved detection and snapshot assembly.
pub fn evidence_field_refs(value: &Value) -> HashSet<EvidenceFieldRef> {
    fn walk(v: &Value, out: &mut HashSet<EvidenceFieldRef>) {
        if let Ok(f) = serde_json::from_value::<EvidenceFieldRef>(v.clone()) {
            let valid = f.adopted().is_some()
                || (f.revision_id.is_none()
                    && f.observation.as_ref().is_some_and(|s| {
                        s.interface_id == f.interface_id
                            && s.environment_id == f.environment_id
                            && s.location == f.location
                            && s.path == f.path
                    }));
            if valid {
                out.insert(f);
                return;
            }
        }
        match v {
            Value::Object(o) => {
                for v in o.values() {
                    walk(v, out);
                }
            }
            Value::Array(a) => {
                for v in a {
                    walk(v, out);
                }
            }
            _ => {}
        }
    }
    let mut out = HashSet::new();
    walk(value, &mut out);
    out
}
pub fn complete_snapshot_fields(snapshot: &KnowledgeSnapshot) -> Vec<KnowledgeField> {
    let mut fields: BTreeMap<_, _> = snapshot_fields(&snapshot.interfaces, &snapshot.annotations)
        .into_iter()
        .map(|f| (f.id.clone(), f))
        .collect();
    if let Some(inputs) = &snapshot.inputs {
        for material in &inputs.observations {
            if material.record.base_revision_id.is_none()
                && material.reconstructed_definition.is_none()
            {
                continue;
            }
            if let Some(definition) = material.definition() {
                let m = &material.record;
                let source = ObservationFieldRef {
                    project_id: m.project_id,
                    interface_id: m.interface_id,
                    environment_id: m.environment_id,
                    ingestion_id: m.ingestion_id,
                    location: String::new(),
                    path: String::new(),
                };
                definition_fields(
                    EvidenceFieldRef {
                        interface_id: m.interface_id,
                        environment_id: m.environment_id,
                        location: String::new(),
                        path: String::new(),
                        revision_id: None,
                        observation: Some(source),
                    },
                    definition,
                    &mut fields,
                );
            }
        }
    }
    for fact in &snapshot.facts {
        for reference in evidence_field_refs(&fact.subject) {
            if reference
                .observation
                .as_ref()
                .is_some_and(|s| s.project_id == snapshot.project_id)
                && snapshot.interfaces.iter().any(|i| {
                    i.interface_id == reference.interface_id
                        && i.environments
                            .iter()
                            .any(|e| e.environment_id == reference.environment_id)
                })
            {
                let id = field_id(&reference);
                fields.entry(id.clone()).or_insert(KnowledgeField {
                    id,
                    reference,
                    schema: Value::Null,
                    schema_pointers: vec![],
                    ancestors: vec![],
                    existing_annotations: vec![],
                });
            }
        }
    }
    fields.into_values().collect()
}
pub fn field_definition<'a>(
    snapshot: &'a KnowledgeSnapshot,
    reference: &EvidenceFieldRef,
) -> Option<&'a Value> {
    if let Some(source) = &reference.observation {
        snapshot
            .inputs
            .as_ref()?
            .observations
            .iter()
            .find(|m| {
                m.record.ingestion_id == source.ingestion_id
                    && m.record.project_id == source.project_id
                    && m.record.interface_id == source.interface_id
                    && m.record.environment_id == source.environment_id
            })?
            .definition()
    } else {
        snapshot
            .interfaces
            .iter()
            .find(|i| i.interface_id == reference.interface_id)?
            .environments
            .iter()
            .find(|e| {
                e.environment_id == reference.environment_id
                    && Some(e.revision_id) == reference.revision_id
            })
            .map(|e| &e.definition)
    }
}
