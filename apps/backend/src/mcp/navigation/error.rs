//! Safe actionable validation details. Never echo query text or source payloads.
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub(crate) struct ReadError {
    code: &'static str,
    details: Option<Value>,
}
impl ReadError {
    pub(crate) fn code(&self) -> &'static str {
        self.code
    }
    pub(crate) fn payload(&self) -> Value {
        let mut out = json!({"error":self.code});
        if let Some(details) = &self.details {
            out["details"] = details.clone();
        }
        out
    }
    pub(crate) fn bounds(
        parameter: &'static str,
        actual: usize,
        minimum: usize,
        maximum: usize,
        suggested: usize,
        unit: &'static str,
    ) -> Self {
        Self {
            code: "INVALID_BOUNDS",
            details: Some(json!({
                "parameter":parameter,"actual":actual,"minimum":minimum,"maximum":maximum,
                "suggested":suggested,"unit":unit,"constraint":"numeric_range","instruction":"Change this parameter within the allowed range. For continuation, copy next.arguments unchanged."
            })),
        }
    }

    pub(super) fn length(
        parameter: &'static str,
        actual: usize,
        minimum: usize,
        maximum: usize,
    ) -> Self {
        Self {
            code: "INVALID_BOUNDS",
            details: Some(json!({
                "parameter":parameter,"constraint":"string_length","actual_length":actual,
                "minimum_length":minimum,"maximum_length":maximum,"unit":"characters",
                "instruction":if parameter == "revision" {"Copy the returned revision unchanged; do not construct or shorten it."} else {"Revise this string within the length bound without losing the requested meaning. No replacement text is inferred."}
            })),
        }
    }
}
impl From<&'static str> for ReadError {
    fn from(code: &'static str) -> Self {
        Self {
            code,
            details: None,
        }
    }
}
impl PartialEq<&str> for ReadError {
    fn eq(&self, other: &&str) -> bool {
        self.code == *other
    }
}
