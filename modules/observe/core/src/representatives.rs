//! Stable value identities and bounded raw retention. No IO or counters here.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const MAX_ADDITIONAL: usize = 15;
pub const MAX_ADDITIONAL_BYTES: usize = 8 * 1024 * 1024;
/// Ignore transport identities/timestamps. Keep actual HTTP values and semantic page context.
pub fn value_id(raw: &Value) -> String {
    let context = raw.get("context");
    let value = json!([
        raw.get("fact"),
        raw.get("site"),
        context.and_then(|v| v.get("page_url")),
        context.and_then(|v| v.get("page_title"))
    ]);
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&value).expect("value"))
    )
}
pub fn retain(first: &Value, saved: &mut Vec<Value>, sample: Value) -> bool {
    let id = value_id(&sample);
    if id == value_id(first) || saved.iter().any(|v| v["id"].as_str() == Some(&id)) {
        return false;
    }
    saved.push(json!({ "id": id, "sample": sample }));
    // First sample is separately retained forever; additional representatives rotate oldest first.
    while saved.len() > MAX_ADDITIONAL
        || (saved.len() > 1
            && serde_json::to_vec(saved).expect("samples").len() > MAX_ADDITIONAL_BYTES)
    {
        saved.remove(0);
    }
    true
}
