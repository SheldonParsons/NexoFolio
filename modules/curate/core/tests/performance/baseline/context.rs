//! Non-sensitive observation provenance and authentication usage, without credentials.
use nexofolio_contracts::{
    knowledge::{AuthenticationUse, Confirmation, EvidenceSource},
    observation::{CanonicalObservation, Fact},
};
use serde_json::Value;

pub(super) fn source(observation: &CanonicalObservation, example: &str) -> EvidenceSource {
    let context = observation.context.as_ref();
    EvidenceSource {
        example: example.into(),
        environment: observation.environment_id,
        page: context
            .and_then(|v| v.get("page_id"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        sequence: context.and_then(|v| v.get("seq")).and_then(Value::as_u64),
        platform: Some(observation.source.platform.clone()),
        ..Default::default()
    }
}

pub fn authentication(example: &str, raw: &Value) -> Vec<AuthenticationUse> {
    let Ok(observation) = serde_json::from_value::<CanonicalObservation>(raw.clone()) else {
        return vec![];
    };
    let Fact::Exchange(exchange) = &observation.fact else {
        return vec![];
    };
    let Some(headers) = &exchange.request.headers else {
        return vec![];
    };
    headers
        .entries
        .iter()
        .filter_map(|(name, value)| {
            if !name.eq_ignore_ascii_case("authorization") {
                return None;
            }
            let (scheme, _) = value.split_once(' ')?;
            let scheme = if scheme.eq_ignore_ascii_case("bearer") {
                "Bearer"
            } else if scheme.eq_ignore_ascii_case("basic") {
                "Basic"
            } else {
                "Unknown"
            };
            Some(AuthenticationUse {
                carrier: "Authorization".into(),
                scheme: scheme.into(),
                credential_ref: format!("redacted:{example}:request.authorization"),
                source: EvidenceSource {
                    pointer: Some("request.headers.Authorization".into()),
                    complete: Some(headers.complete),
                    ..source(&observation, example)
                },
                confirmation: Confirmation::Observed,
            })
        })
        .collect()
}
