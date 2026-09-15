use super::*;
pub(super) fn key(value: &Value) -> String {
    Sha256::digest(serde_json::to_vec(value).expect("serializes"))
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn field_id(reference: &FieldRef) -> String {
    format!(
        "f_{}",
        &key(&serde_json::to_value(reference).unwrap())[..32]
    )
}
pub(super) fn esc(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}
fn child_ref(base: &FieldRef, path: String) -> FieldRef {
    let mut f = base.clone();
    f.path = path;
    f
}
fn local_schema(schema: &Value, reference: &FieldRef) -> Value {
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
    reference: FieldRef,
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
pub fn snapshot_fields(
    interfaces: &[CatalogInterface],
    annotations: &[SemanticAnnotation],
) -> Vec<KnowledgeField> {
    let mut fields = BTreeMap::new();
    for interface in interfaces {
        for env in &interface.environments {
            let base = FieldRef {
                interface_id: interface.interface_id,
                environment_id: env.environment_id,
                revision_id: env.revision_id,
                location: String::new(),
                path: String::new(),
            };
            let d = &env.definition;
            for side in ["request", "response"] {
                let mut body = base.clone();
                body.location = format!("{side}.body");
                let schema = &d[side]["body"]["observed_schema"];
                collect_schema(
                    schema,
                    body,
                    format!("/{side}/body/observed_schema"),
                    vec![],
                    &mut fields,
                );
                for (index, header) in d[side]["headers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    if let Some(name) = header["name"].as_str() {
                        let mut f = base.clone();
                        f.location = format!("{side}.header");
                        f.path = format!("/{}", esc(name));
                        collect_schema(
                            &header["observed_schema"],
                            f,
                            format!("/{side}/headers/{index}/observed_schema"),
                            vec![],
                            &mut fields,
                        );
                    }
                }
            }
            for (index, parameter) in d["request"]["parameters"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                if let (Some(name), Some(location)) =
                    (parameter["name"].as_str(), parameter["in"].as_str())
                {
                    let mut f = base.clone();
                    f.location = format!("request.{location}");
                    f.path = format!("/{}", esc(name));
                    collect_schema(
                        &parameter["observed_schema"],
                        f,
                        format!("/request/parameters/{index}/observed_schema"),
                        vec![],
                        &mut fields,
                    );
                }
            }
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
