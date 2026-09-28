//! Content hash of a batch, stable under re-serialisation: keys sorted, no
//! whitespace. A client that retries with the same content but a different
//! key order or formatting is not mistaken for reusing the batch ID.

use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) fn content_hash(value: &Value) -> [u8; 32] {
    let mut hasher = Sha256::new();
    write(&mut hasher, value);
    hasher.finalize().into()
}

fn write(hasher: &mut Sha256, value: &Value) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            hasher.update(b"{");
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    hasher.update(b",");
                }
                hasher.update(Value::String(key.clone()).to_string());
                hasher.update(b":");
                write(hasher, &map[key]);
            }
            hasher.update(b"}");
        }
        Value::Array(items) => {
            hasher.update(b"[");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    hasher.update(b",");
                }
                write(hasher, item);
            }
            hasher.update(b"]");
        }
        // Compact JSON of a scalar is already canonical.
        scalar => hasher.update(scalar.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_order_and_whitespace_do_not_matter() {
        let a: Value = serde_json::from_str(r#"{"b":[1,{"y":"é","x":null}],"a":true}"#).unwrap();
        let b: Value = serde_json::from_str(
            "{ \"a\": true,\n \"b\": [1, {\"x\": null, \"y\": \"\\u00e9\"}] }",
        )
        .unwrap();
        assert_eq!(content_hash(&a), content_hash(&b));
        let c: Value = serde_json::from_str(r#"{"a":true,"b":[{"x":null,"y":"é"},1]}"#).unwrap();
        assert_ne!(content_hash(&a), content_hash(&c), "array order matters");
    }
}
