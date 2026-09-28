//! The shape of a JSON value: which types appear where. Values are dropped.

use std::collections::{BTreeMap, BTreeSet};

use nexofolio_contracts::endpoint::{PathSegment, ValueType};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Empty `types` means unknown: merging it with any shape gives that shape.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shape {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub types: BTreeSet<ValueType>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, FieldShape>,
    /// Merged shape of every element; `None` for empty arrays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<Shape>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldShape {
    pub shape: Shape,
    /// Present in every object merged here, e.g. in every array element.
    pub always: bool,
}

/// Where a field stands in one shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Present,
    /// The parent is an object without it.
    Absent,
    /// Present in some merged objects only.
    Mixed,
    /// The shape says nothing: parent missing, not an object, or empty array.
    Unknown,
}

impl Shape {
    pub fn of(value: &Value) -> Self {
        let mut shape = Self::default();
        match value {
            Value::Null => {
                shape.types.insert(ValueType::Null);
            }
            Value::Bool(_) => {
                shape.types.insert(ValueType::Boolean);
            }
            Value::Number(_) => {
                shape.types.insert(ValueType::Number);
            }
            Value::String(_) => {
                shape.types.insert(ValueType::String);
            }
            Value::Array(values) => {
                shape.types.insert(ValueType::Array);
                shape.items = values
                    .iter()
                    .map(Self::of)
                    .reduce(Self::merge)
                    .map(Box::new);
            }
            Value::Object(entries) => {
                shape.types.insert(ValueType::Object);
                shape.fields = entries
                    .iter()
                    .map(|(key, value)| (key.clone(), FieldShape::always(Self::of(value))))
                    .collect();
            }
        }
        shape
    }

    /// An object whose keys all hold strings, e.g. a query or a form.
    pub fn of_keys<'a>(keys: impl IntoIterator<Item = &'a str>) -> Self {
        let mut string = Self::default();
        string.types.insert(ValueType::String);
        Self {
            types: BTreeSet::from([ValueType::Object]),
            fields: keys
                .into_iter()
                .map(|key| (key.to_owned(), FieldShape::always(string.clone())))
                .collect(),
            items: None,
        }
    }

    pub fn merge(self, other: Self) -> Self {
        let self_object = self.types.contains(&ValueType::Object);
        let other_object = other.types.contains(&ValueType::Object);
        // A key one side lacks is optional only if that side was an object.
        let mut fields = self.fields;
        for (key, field) in fields.iter_mut() {
            field.always &= !other_object || other.fields.contains_key(key);
        }
        for (key, theirs) in other.fields {
            let merged = match fields.remove(&key) {
                Some(ours) => FieldShape {
                    shape: ours.shape.merge(theirs.shape),
                    always: ours.always && theirs.always,
                },
                None => FieldShape {
                    shape: theirs.shape,
                    always: theirs.always && !self_object,
                },
            };
            fields.insert(key, merged);
        }
        let items = match (self.items, other.items) {
            (Some(a), Some(b)) => Some(Box::new(a.merge(*b))),
            (a, b) => a.or(b),
        };
        Self {
            types: self.types.into_iter().chain(other.types).collect(),
            fields,
            items,
        }
    }

    /// The shape at `path` and whether it is there.
    pub fn at(&self, path: &[PathSegment]) -> (Presence, Option<&Shape>) {
        let Some((last, parents)) = path.split_last() else {
            return (Presence::Present, Some(self));
        };
        let mut node = self;
        for segment in parents {
            match node.child(segment) {
                Some((_, child)) => node = child,
                None => return (Presence::Unknown, None),
            }
        }
        match node.child(last) {
            Some((true, child)) => (Presence::Present, Some(child)),
            Some((false, child)) => (Presence::Mixed, Some(child)),
            None if matches!(last, PathSegment::Key(_))
                && node.types.contains(&ValueType::Object) =>
            {
                (Presence::Absent, None)
            }
            None => (Presence::Unknown, None),
        }
    }

    fn child(&self, segment: &PathSegment) -> Option<(bool, &Shape)> {
        match segment {
            PathSegment::Key(key) => self.fields.get(key).map(|f| (f.always, &f.shape)),
            PathSegment::Items => self.items.as_deref().map(|items| (true, items)),
        }
    }

    /// Every field path below this shape, parents first.
    pub fn paths(&self) -> Vec<Vec<PathSegment>> {
        let mut paths = Vec::new();
        self.collect_paths(&mut Vec::new(), &mut paths);
        paths
    }

    fn collect_paths(&self, prefix: &mut Vec<PathSegment>, out: &mut Vec<Vec<PathSegment>>) {
        let children = self
            .fields
            .iter()
            .map(|(key, field)| (PathSegment::Key(key.clone()), &field.shape))
            .chain(
                self.items
                    .as_deref()
                    .map(|items| (PathSegment::Items, items)),
            );
        for (segment, shape) in children {
            prefix.push(segment);
            out.push(prefix.clone());
            shape.collect_paths(prefix, out);
            prefix.pop();
        }
    }
}

impl FieldShape {
    fn always(shape: Shape) -> Self {
        Self {
            shape,
            always: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(name: &str) -> PathSegment {
        PathSegment::Key(name.into())
    }

    #[test]
    fn keeps_types_not_values() {
        let shape = Shape::of(&json!({ "id": 1, "name": "a", "tags": ["x"], "note": null }));
        assert_eq!(
            shape,
            Shape::of(&json!({ "id": 2, "name": "b", "tags": ["y", "z"], "note": null }))
        );
        assert_eq!(
            shape
                .at(&[key("tags"), PathSegment::Items])
                .1
                .unwrap()
                .types,
            BTreeSet::from([ValueType::String])
        );
    }

    #[test]
    fn array_elements_merge_and_track_missing_keys() {
        let shape = Shape::of(&json!([{ "sku": "a", "qty": 1 }, { "sku": "b" }]));
        let items = [PathSegment::Items];
        assert_eq!(
            shape.at(&[items[0].clone(), key("sku")]).0,
            Presence::Present
        );
        assert_eq!(shape.at(&[items[0].clone(), key("qty")]).0, Presence::Mixed);
        assert_eq!(
            shape.at(&[items[0].clone(), key("gone")]).0,
            Presence::Absent
        );
    }

    #[test]
    fn empty_arrays_are_unknown_and_merge_away() {
        let empty = Shape::of(&json!({ "list": [] }));
        let path = [key("list"), PathSegment::Items, key("sku")];
        assert_eq!(empty.at(&path).0, Presence::Unknown);
        let full = Shape::of(&json!({ "list": [{ "sku": "a" }] }));
        let merged = empty.merge(full.clone());
        assert_eq!(merged, full, "an empty array adds nothing");
    }

    #[test]
    fn a_null_parent_does_not_make_children_optional() {
        let merged = Shape::of(&json!({ "a": 1 })).merge(Shape::of(&json!(null)));
        assert_eq!(merged.at(&[key("a")]).0, Presence::Present);
        let merged = Shape::of(&json!({ "a": 1 })).merge(Shape::of(&json!({})));
        assert_eq!(merged.at(&[key("a")]).0, Presence::Mixed);
        let merged = Shape::of(&json!({})).merge(Shape::of(&json!({ "a": 1 })));
        assert_eq!(merged.at(&[key("a")]).0, Presence::Mixed);
    }

    #[test]
    fn missing_parents_say_nothing_about_children() {
        let shape = Shape::of(&json!({ "data": null }));
        assert_eq!(shape.at(&[key("data"), key("id")]).0, Presence::Unknown);
        assert_eq!(shape.at(&[key("other"), key("id")]).0, Presence::Unknown);
        assert_eq!(shape.at(&[key("other")]).0, Presence::Absent);
    }

    #[test]
    fn lists_paths_parents_first() {
        let shape = Shape::of(&json!({ "list": [{ "sku": "a" }] }));
        assert_eq!(
            shape.paths(),
            vec![
                vec![key("list")],
                vec![key("list"), PathSegment::Items],
                vec![key("list"), PathSegment::Items, key("sku")],
            ]
        );
    }
}
