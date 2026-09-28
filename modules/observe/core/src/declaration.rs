//! What a declaration (OpenAPI and alike) says about each field.

use std::collections::BTreeMap;

use nexofolio_contracts::endpoint::{
    DeclaredField, FieldLocation, FieldPath, PathSegment, ValueType,
};
use nexofolio_contracts::observation::{DeclaredParameter, HttpDeclaration};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type Key = (FieldLocation, FieldPath);

/// The stored form of one declaration: every field it names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredFields(pub Vec<(FieldLocation, FieldPath, DeclaredField)>);

impl DeclaredFields {
    pub fn of(declaration: &HttpDeclaration) -> Self {
        let mut fields = Fields::new();
        let parameters = [
            (FieldLocation::Path, &declaration.request.path_params),
            (FieldLocation::Query, &declaration.request.query),
        ];
        for (location, parameters) in parameters {
            for (name, parameter) in parameters {
                parameter_field(&mut fields, location, name, parameter);
            }
        }
        if let Some(schema) = declaration
            .request
            .body
            .as_ref()
            .and_then(|b| b.schema.as_ref())
        {
            walk(
                &mut fields,
                FieldLocation::RequestBody,
                &mut Vec::new(),
                schema,
            );
        }
        for (status, response) in &declaration.responses {
            let (Ok(status), Some(schema)) = (status.parse::<u16>(), &response.schema) else {
                continue;
            };
            walk(
                &mut fields,
                FieldLocation::ResponseBody { status },
                &mut Vec::new(),
                schema,
            );
        }
        Self(
            fields
                .into_iter()
                .map(|((location, path), field)| (location, path, field))
                .collect(),
        )
    }
}

/// Fields of several declarations; where they disagree the newest wins.
/// Takes `(updated_at, fields)` in any order.
pub fn merge<T: Ord>(mut declarations: Vec<(T, DeclaredFields)>) -> BTreeMap<Key, DeclaredField> {
    declarations.sort_by(|a, b| a.0.cmp(&b.0));
    let mut merged = BTreeMap::new();
    for (_, DeclaredFields(fields)) in declarations {
        for (location, path, field) in fields {
            merged.insert((location, path), field);
        }
    }
    merged
}

type Fields = BTreeMap<Key, DeclaredField>;

fn parameter_field(
    fields: &mut Fields,
    location: FieldLocation,
    name: &str,
    parameter: &DeclaredParameter,
) {
    let path = FieldPath(vec![PathSegment::Key(name.to_owned())]);
    let types = parameter.schema.as_ref().map(types_of).unwrap_or_default();
    fields.insert(
        (location, path),
        DeclaredField {
            required: parameter.required,
            types,
        },
    );
}

/// Records every field below `schema`. The root itself is not a field.
fn walk(fields: &mut Fields, location: FieldLocation, path: &mut Vec<PathSegment>, schema: &Value) {
    for (keyword, all_required) in [("oneOf", false), ("anyOf", false), ("allOf", true)] {
        let Some(alternatives) = schema.get(keyword).and_then(Value::as_array) else {
            continue;
        };
        let mut combined: Option<Fields> = None;
        for alternative in alternatives {
            let mut own = Fields::new();
            walk(&mut own, location, path, alternative);
            combined = Some(match combined {
                None => own,
                Some(before) => combine(before, own, all_required),
            });
        }
        for (key, field) in combined.unwrap_or_default() {
            let merged = match fields.remove(&key) {
                Some(existing) => union(existing, field, true),
                None => field,
            };
            fields.insert(key, merged);
        }
    }
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            path.push(PathSegment::Key(name.clone()));
            record(
                fields,
                location,
                path,
                property,
                required.contains(&name.as_str()),
            );
            walk(fields, location, path, property);
            path.pop();
        }
    }
    if let Some(items) = schema.get("items").filter(|items| items.is_object()) {
        path.push(PathSegment::Items);
        record(fields, location, path, items, false);
        walk(fields, location, path, items);
        path.pop();
    }
}

fn record(
    fields: &mut Fields,
    location: FieldLocation,
    path: &[PathSegment],
    schema: &Value,
    required: bool,
) {
    let field = DeclaredField {
        required,
        types: types_of(schema),
    };
    let key = (location, FieldPath(path.to_vec()));
    let merged = match fields.remove(&key) {
        Some(existing) => union(existing, field, true),
        None => field,
    };
    fields.insert(key, merged);
}

/// Fields of two alternatives. With `oneOf`/`anyOf` a field is required only
/// if both require it; with `allOf` if either does.
fn combine(a: Fields, mut b: Fields, all_required: bool) -> Fields {
    let mut out = Fields::new();
    for (key, field) in a {
        let merged = match b.remove(&key) {
            Some(other) => union(field, other, all_required),
            None => DeclaredField {
                required: field.required && all_required,
                ..field
            },
        };
        out.insert(key, merged);
    }
    for (key, field) in b {
        out.insert(
            key,
            DeclaredField {
                required: field.required && all_required,
                ..field
            },
        );
    }
    out
}

fn union(a: DeclaredField, b: DeclaredField, either_required: bool) -> DeclaredField {
    let mut types = a.types;
    for kind in b.types {
        if !types.contains(&kind) {
            types.push(kind);
        }
    }
    types.sort();
    DeclaredField {
        required: if either_required {
            a.required || b.required
        } else {
            a.required && b.required
        },
        types,
    }
}

/// `type` (string or list), `nullable`, and the types of `oneOf`/`anyOf`/`allOf`.
fn types_of(schema: &Value) -> Vec<ValueType> {
    let mut types: Vec<ValueType> = match schema.get("type") {
        Some(Value::String(name)) => type_named(name).into_iter().collect(),
        Some(Value::Array(names)) => names
            .iter()
            .filter_map(Value::as_str)
            .filter_map(type_named)
            .collect(),
        _ => Vec::new(),
    };
    if schema.get("nullable") == Some(&Value::Bool(true)) {
        types.push(ValueType::Null);
    }
    for keyword in ["oneOf", "anyOf", "allOf"] {
        for alternative in schema
            .get(keyword)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            types.extend(types_of(alternative));
        }
    }
    if types.is_empty() && schema.get("properties").is_some() {
        types.push(ValueType::Object);
    }
    types.sort();
    types.dedup();
    types
}

fn type_named(name: &str) -> Option<ValueType> {
    Some(match name {
        "null" => ValueType::Null,
        "boolean" => ValueType::Boolean,
        "integer" | "number" => ValueType::Number,
        "string" => ValueType::String,
        "object" => ValueType::Object,
        "array" => ValueType::Array,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexofolio_common::ProjectId;
    use nexofolio_contracts::observation::{DeclaredBody, DeclaredResponse, Fact};
    use nexofolio_contracts::testing::sample_declaration;
    use serde_json::json;

    fn key(name: &str) -> PathSegment {
        PathSegment::Key(name.into())
    }

    fn declaration() -> HttpDeclaration {
        let Fact::Declaration(declaration) = sample_declaration(ProjectId::new()).fact else {
            unreachable!()
        };
        declaration
    }

    fn get<'a>(
        fields: &'a DeclaredFields,
        location: FieldLocation,
        path: &[PathSegment],
    ) -> Option<&'a DeclaredField> {
        fields
            .0
            .iter()
            .find(|(l, p, _)| *l == location && p.0 == path)
            .map(|(_, _, field)| field)
    }

    #[test]
    fn walks_parameters_bodies_and_responses() {
        let mut declared = declaration();
        declared.request.body = Some(DeclaredBody {
            schema: Some(json!({
                "type": "object",
                "required": ["items"],
                "properties": {
                    "items": { "type": "array", "items": {
                        "type": "object", "required": ["sku"],
                        "properties": { "sku": { "type": "string" }, "qty": { "type": "integer", "nullable": true } }
                    }}
                }
            })),
            ..DeclaredBody::default()
        });
        declared.responses.insert(
            "200".into(),
            DeclaredResponse {
                schema: Some(json!({ "properties": { "id": { "type": ["integer", "null"] } } })),
                ..DeclaredResponse::default()
            },
        );
        declared.responses.insert(
            "default".into(),
            DeclaredResponse {
                schema: Some(json!({ "properties": { "error": { "type": "string" } } })),
                ..DeclaredResponse::default()
            },
        );
        let fields = DeclaredFields::of(&declared);
        let order_id = get(&fields, FieldLocation::Path, &[key("orderId")]).unwrap();
        assert_eq!(
            order_id,
            &DeclaredField {
                required: true,
                types: vec![ValueType::Number]
            }
        );
        let body = FieldLocation::RequestBody;
        assert!(get(&fields, body, &[key("items")]).unwrap().required);
        assert!(
            !get(&fields, body, &[key("items"), PathSegment::Items])
                .unwrap()
                .required
        );
        assert!(
            get(
                &fields,
                body,
                &[key("items"), PathSegment::Items, key("sku")]
            )
            .unwrap()
            .required
        );
        let qty = get(
            &fields,
            body,
            &[key("items"), PathSegment::Items, key("qty")],
        )
        .unwrap();
        assert_eq!(qty.types, vec![ValueType::Null, ValueType::Number]);
        let id = get(
            &fields,
            FieldLocation::ResponseBody { status: 200 },
            &[key("id")],
        )
        .unwrap();
        assert_eq!(
            (id.required, id.types.clone()),
            (false, vec![ValueType::Null, ValueType::Number])
        );
        assert_eq!(fields.0.len(), 6, "non-numeric responses are skipped");
    }

    #[test]
    fn alternatives_require_what_every_branch_requires() {
        let mut declared = declaration();
        declared.responses.insert("200".into(), DeclaredResponse {
            schema: Some(json!({
                "oneOf": [
                    { "type": "object", "required": ["id", "a"], "properties": { "id": { "type": "string" }, "a": { "type": "string" } } },
                    { "type": "object", "required": ["id"], "properties": { "id": { "type": "integer" } } }
                ],
                "allOf": [
                    { "required": ["x"], "properties": { "x": { "type": "string" } } },
                    { "properties": { "y": { "type": "string" } } }
                ]
            })),
            ..DeclaredResponse::default()
        });
        let fields = DeclaredFields::of(&declared);
        let at = |name| {
            get(
                &fields,
                FieldLocation::ResponseBody { status: 200 },
                &[key(name)],
            )
            .unwrap()
        };
        assert_eq!(
            at("id"),
            &DeclaredField {
                required: true,
                types: vec![ValueType::Number, ValueType::String]
            }
        );
        assert!(!at("a").required);
        assert!(at("x").required);
        assert!(!at("y").required);
    }

    #[test]
    fn newest_declaration_wins() {
        let path = FieldPath(vec![key("id")]);
        let field = |required| DeclaredField {
            required,
            types: vec![],
        };
        let old = DeclaredFields(vec![(FieldLocation::Query, path.clone(), field(true))]);
        let new = DeclaredFields(vec![(FieldLocation::Query, path.clone(), field(false))]);
        let merged = merge(vec![(2, new), (1, old)]);
        assert!(!merged[&(FieldLocation::Query, path)].required);
    }
}
