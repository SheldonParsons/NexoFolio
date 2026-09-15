//! Deterministic path recognition. Pure segment checks only; no model or database calls.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const PATH_RULE_VERSION: &str = "path-template-1";
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathPolicy {
    pub enabled: bool,
    pub numeric_segments: bool,
    /// Segment-boundary prefixes which retain literal paths, including "/" for all paths.
    pub literal_prefixes: Vec<String>,
}
impl Default for PathPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            numeric_segments: true,
            literal_prefixes: vec![],
        }
    }
}
impl PathPolicy {
    pub fn valid(&self) -> bool {
        self.literal_prefixes.len() <= 100
            && self.literal_prefixes.iter().all(|p| {
                p.starts_with('/')
                    && p.len() <= 4096
                    && !p.contains(['?', '#'])
                    && !p.chars().any(char::is_control)
            })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathParameterKind {
    Uuid,
    Hex32,
    Hex64,
    Integer,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct PathParameter {
    pub name: String,
    pub segment_index: usize,
    pub kind: PathParameterKind,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct PathIdentity {
    pub rule_version: String,
    pub template: String,
    pub parameters: Vec<PathParameter>,
}
pub fn identify_path(path: &str, policy: &PathPolicy) -> PathIdentity {
    let mut result = PathIdentity {
        rule_version: PATH_RULE_VERSION.into(),
        template: path.into(),
        parameters: vec![],
    };
    if !policy.enabled
        || policy.literal_prefixes.iter().any(|prefix| {
            let prefix = prefix.trim_end_matches('/');
            prefix.is_empty()
                || path == prefix
                || path
                    .strip_prefix(prefix)
                    .is_some_and(|tail| tail.starts_with('/'))
        })
    {
        return result;
    }
    let segments: Vec<_> = path
        .split('/')
        .enumerate()
        .map(|(index, segment)| {
            let bytes = segment.as_bytes();
            let uuid = bytes.len() == 36
                && [8, 13, 18, 23].iter().all(|i| bytes[*i] == b'-')
                && bytes
                    .iter()
                    .enumerate()
                    .all(|(i, b)| [8, 13, 18, 23].contains(&i) || b.is_ascii_hexdigit());
            let kind = if uuid {
                Some(PathParameterKind::Uuid)
            } else if !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit) {
                policy
                    .numeric_segments
                    .then_some(PathParameterKind::Integer)
            } else if bytes.iter().all(u8::is_ascii_hexdigit) {
                match bytes.len() {
                    32 => Some(PathParameterKind::Hex32),
                    64 => Some(PathParameterKind::Hex64),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(kind) = kind {
                let name = format!("param{}", result.parameters.len() + 1);
                result.parameters.push(PathParameter {
                    name: name.clone(),
                    segment_index: index,
                    kind,
                });
                format!("{{{name}}}")
            } else {
                segment.into()
            }
        })
        .collect();
    result.template = segments.join("/");
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_sample_recognizes_numbers_uuid_and_hex_without_touching_literals() {
        for (path, expected) in [
            ("/users/123", "/users/{param1}"),
            ("/v1/reports/2025", "/v1/reports/{param1}"),
            (
                "/users/550e8400-e29b-41d4-a716-446655440000/orders/01/",
                "/users/{param1}/orders/{param2}/",
            ),
            ("/find/abcdefabcdefabcdefabcdefabcdefab", "/find/{param1}"),
            (
                "/me/search/latest/2025-01-02/%31%32/-1",
                "/me/search/latest/2025-01-02/%31%32/-1",
            ),
        ] {
            assert_eq!(
                identify_path(path, &PathPolicy::default()).template,
                expected
            );
        }
    }
    #[test]
    fn overrides_have_segment_boundaries_and_do_not_trim_case_or_slashes() {
        let p = PathPolicy {
            literal_prefixes: vec!["/reports".into()],
            ..PathPolicy::default()
        };
        assert_eq!(identify_path("/reports/2025", &p).template, "/reports/2025");
        assert_eq!(
            identify_path("/reports2/2025", &p).template,
            "/reports2/{param1}"
        );
        let p = PathPolicy {
            enabled: false,
            ..p
        };
        assert_eq!(identify_path("/USERS//12/", &p).template, "/USERS//12/");
        let p = PathPolicy {
            numeric_segments: false,
            ..PathPolicy::default()
        };
        assert_eq!(identify_path("/users/123", &p).template, "/users/123");
        let long = format!("/users/{}", "1".repeat(32));
        assert_eq!(identify_path(&long, &p).template, long);
    }
}
