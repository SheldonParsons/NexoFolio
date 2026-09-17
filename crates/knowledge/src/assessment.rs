//! Field-level interpretation; persistence and publication do not live here.
use nexofolio_contracts::{
    AssessmentKind as Kind, StructuralAssessment, StructuralFinding, compare_observed_schemas,
};
use serde_json::{Value, json};

fn add(
    out: &mut Vec<StructuralFinding>,
    kind: Kind,
    location: &str,
    reason: &str,
    a: Option<&Value>,
    b: Option<&Value>,
) {
    out.push(StructuralFinding {
        kind,
        location: location.into(),
        path: String::new(),
        reason: reason.into(),
        before: a.cloned(),
        observed: b.cloned(),
    });
}
fn metadata(out: &mut Vec<StructuralFinding>, location: &str, reason: &str, a: &Value, b: &Value) {
    if a == b {
        return;
    }
    let unknown = |v: &Value| v.is_null() || v == "";
    let kind = if unknown(b) {
        Kind::Insufficient
    } else if unknown(a) {
        Kind::Enrichment
    } else {
        Kind::Difference
    };
    add(out, kind, location, reason, Some(a), Some(b));
}
fn named(values: &Value, parameter: Option<&str>) -> Value {
    let props: serde_json::Map<_, _> = values
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| parameter.is_none_or(|kind| v["in"] == kind))
        .filter_map(|v| Some((v["name"].as_str()?.into(), v["observed_schema"].clone())))
        .collect();
    json!({"type":"object","properties":props})
}
pub fn assess_definition(known: Option<&Value>, incoming: &Value) -> StructuralAssessment {
    let mut out = vec![];
    if let Some(a) = known
        && (a["method"] != incoming["method"] || a["path"] != incoming["path"])
    {
        add(
            &mut out,
            Kind::Insufficient,
            "interface",
            "IDENTITY_MISMATCH",
            None,
            None,
        );
        return StructuralAssessment::new(false, out);
    }
    for side in ["request", "response"] {
        let b = &incoming[side];
        let a = known.map(|k| &k[side]);
        if let Some(a) = a {
            out.extend(compare_observed_schemas(
                &named(&a["headers"], None),
                &named(&b["headers"], None),
                &format!("{side}.header"),
            ));
            if side == "request" {
                for part in ["query", "path"] {
                    out.extend(compare_observed_schemas(
                        &named(&a["parameters"], Some(part)),
                        &named(&b["parameters"], Some(part)),
                        &format!("request.{part}"),
                    ));
                }
            }
            if side == "response" {
                metadata(
                    &mut out,
                    "response.status",
                    "RESPONSE_STATUS_OBSERVED",
                    &a["status"],
                    &b["status"],
                );
            }
        }
        let body = &b["body"];
        let previous = a.map(|a| &a["body"]);
        let loc = format!("{side}.body");
        if let Some(a) = previous {
            metadata(
                &mut out,
                &format!("{side}.media_type"),
                "MEDIA_TYPE_OBSERVED",
                &a["media_type"],
                &body["media_type"],
            );
        }
        match body["state"].as_str() {
            Some("none") => {
                if previous.is_some_and(|a| a["state"] != "none") {
                    add(
                        &mut out,
                        Kind::Insufficient,
                        &loc,
                        "BODY_NOT_OBSERVED",
                        None,
                        None,
                    );
                }
            }
            Some("complete") if !body["observed_schema"].is_null() => {
                if let Some(a) =
                    previous.filter(|a| a["state"] == "complete" && !a["observed_schema"].is_null())
                {
                    out.extend(compare_observed_schemas(
                        &a["observed_schema"],
                        &body["observed_schema"],
                        &loc,
                    ));
                } else if known.is_some() {
                    add(
                        &mut out,
                        Kind::Enrichment,
                        &loc,
                        "BODY_STRUCTURE_OBSERVED",
                        previous.map(|a| &a["observed_schema"]),
                        Some(&body["observed_schema"]),
                    );
                } else {
                    out.extend(compare_observed_schemas(
                        &body["observed_schema"],
                        &body["observed_schema"],
                        &loc,
                    ));
                }
            }
            _ => add(
                &mut out,
                Kind::Insufficient,
                &loc,
                "BODY_STRUCTURE_UNAVAILABLE",
                None,
                None,
            ),
        }
    }
    for (definition, prefix) in [(known, "BASELINE"), (Some(incoming), "OBSERVATION")] {
        for note in definition
            .into_iter()
            .flat_map(|d| d["limitations"].as_array())
            .flatten()
            .filter_map(Value::as_str)
        {
            if note == "STRUCTURE_EXTRACTION_LIMIT"
                || note == "REQUEST_URL_TRUNCATED"
                || note.ends_with("HEADERS_UNREADABLE")
                || note.ends_with("HEADERS_TRUNCATED")
            {
                add(
                    &mut out,
                    Kind::Insufficient,
                    "observation",
                    &format!("{prefix}_{note}"),
                    None,
                    None,
                );
            }
        }
    }
    if incoming["response"]["capture_state"] != "complete" {
        add(
            &mut out,
            Kind::Insufficient,
            "response",
            "RESPONSE_CAPTURE_INCOMPLETE",
            None,
            None,
        );
    }
    // v1 had only a global budget marker; it cannot prove that an absent field
    // or variant was ever read. v2's scoped markers keep unrelated changes usable.
    if known.is_some_and(|a| {
        a["extractor_version"] == "observed-http-1"
            && a["limitations"]
                .as_array()
                .is_some_and(|v| v.contains(&json!("STRUCTURE_EXTRACTION_LIMIT")))
    }) {
        for f in &mut out {
            if f.location.ends_with(".body")
                && ["FIELD_FIRST_OBSERVED", "OBSERVED_VARIANT_DIFFERS"].contains(&f.reason.as_str())
            {
                f.kind = Kind::Enrichment;
                f.reason = "BASELINE_UNREAD_STRUCTURE_OBSERVED".into();
            }
        }
    }
    StructuralAssessment::new(known.is_none(), out)
}

/// Stable read boundary for evidence and snapshot consumers; never recomputes historical results.
#[async_trait::async_trait]
pub trait AssessmentReader: Send + Sync {
    async fn assessment(
        &self,
        user: nexofolio_contracts::UserId,
        project: nexofolio_contracts::ProjectId,
        ingestion_id: uuid::Uuid,
    ) -> nexofolio_contracts::Result<nexofolio_contracts::ObservationAssessment>;
    async fn assessments(
        &self,
        user: nexofolio_contracts::UserId,
        project: nexofolio_contracts::ProjectId,
        interface: nexofolio_contracts::InterfaceId,
        query: crate::DocumentQuery,
    ) -> nexofolio_contracts::Result<nexofolio_contracts::ObservationAssessmentPage>;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn definition(body: Value) -> Value {
        let mut raw: Value = serde_json::from_str(include_str!(
            "../../../contracts/ingestion/fixtures/http-batch.json"
        ))
        .unwrap();
        raw["records"][0]["payload"]["response"]["body"]["content"] = json!(body.to_string());
        serde_json::to_value(crate::extract_observed(&raw["records"][0]).unwrap()).unwrap()
    }
    #[test]
    fn mixed_findings_keep_enrichment_difference_and_missing_evidence() {
        let a = definition(json!({"items":[],"price":1,"a/b":{"~":1},"optional":true}));
        let b = definition(json!({"items":[{"id":1}],"price":"1","a/b":{"~":false}}));
        let result = assess_definition(Some(&a), &b);
        assert_eq!(
            result.categories,
            [Kind::Enrichment, Kind::Difference, Kind::Insufficient]
        );
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.path == "/items/*" && f.kind == Kind::Enrichment)
        );
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.path == "/optional" && f.reason == "FIELD_NOT_OBSERVED")
        );
        assert!(result.findings.iter().any(|f| f.path == "/a~1b/~0"));
    }
    #[test]
    fn null_observation_and_incomplete_capture_are_not_the_same() {
        let a = definition(json!({"value":null}));
        let b = definition(json!({"value":123}));
        assert!(assess_definition(Some(&a), &a).is_duplicate());
        assert_eq!(
            assess_definition(Some(&a), &b).categories,
            [Kind::Enrichment]
        );
        assert_eq!(
            assess_definition(Some(&b), &a).categories,
            [Kind::Insufficient]
        );
        for state in ["truncated", "unreadable"] {
            let mut partial = b.clone();
            partial["response"]["body"]["state"] = json!(state);
            partial["response"]["body"]["observed_schema"] = Value::Null;
            assert_eq!(
                assess_definition(Some(&partial), &partial).categories,
                [Kind::Insufficient]
            );
        }
        let mut legacy = b.clone();
        legacy["limitations"]
            .as_array_mut()
            .unwrap()
            .push(json!("STRUCTURE_EXTRACTION_LIMIT"));
        assert!(!assess_definition(Some(&legacy), &legacy).is_duplicate());
    }
    #[test]
    fn budget_exhaustion_does_not_hide_a_readable_sibling_or_prove_repetition() {
        let a = definition(json!({"a":1,"data":vec![0;100_001],"z":1}));
        let b = definition(json!({"a":"changed","data":vec![0;100_001],"z":2}));
        assert!(!assess_definition(Some(&a), &a).is_duplicate());
        let result = assess_definition(Some(&a), &b);
        assert!(result.categories.contains(&Kind::Difference));
        assert!(result.categories.contains(&Kind::Insufficient));
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.path == "/a" && f.kind == Kind::Difference)
        );
    }
    #[test]
    fn evidence_and_snapshot_consumers_share_observation_identity_not_baseline_fields() {
        use nexofolio_contracts::*;
        let a = definition(json!({"items":[]}));
        let b = definition(json!({"items":[{"id":123}]}));
        let material = ObservationAssessment {
            project_id: ProjectId::new(),
            interface_id: InterfaceId::new(),
            environment_id: EnvironmentId::new(),
            ingestion_id: uuid::Uuid::new_v4(),
            base_revision_id: Some(RevisionId::new()),
            extractor_version: Some("observed-http-2".into()),
            assessment: Some(assess_definition(Some(&a), &b)),
            incoming_definition: Some(b),
        };
        // Wire round-trip is part of the boundary, not an internal concrete adapter dependency.
        let decoded: ObservationAssessment =
            serde_json::from_value(serde_json::to_value(&material).unwrap()).unwrap();
        let evidence_consumer = |m: &ObservationAssessment| {
            m.assessment
                .as_ref()
                .unwrap()
                .findings
                .iter()
                .filter(|f| f.observed.is_some())
                .map(|f| m.observed_field(f))
                .collect::<Vec<_>>()
        };
        let refs = evidence_consumer(&decoded);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].ingestion_id, material.ingestion_id);
        assert!(
            serde_json::to_value(&refs[0])
                .unwrap()
                .get("revision_id")
                .is_none()
        );
        // Snapshot consumer keeps current and unadopted material separate.
        let snapshot_consumer = |m: &ObservationAssessment| json!({"basis":m.base_revision_id,"observation_fields":evidence_consumer(m),"observed_definition":m.incoming_definition});
        let snapshot = snapshot_consumer(&decoded);
        assert_eq!(snapshot["observation_fields"][0]["path"], "/items/*");
        assert_eq!(
            a["response"]["body"]["observed_schema"]["properties"]["items"]["items"]["unknown"],
            true
        );
    }
    #[test]
    fn legacy_budget_gaps_do_not_turn_unread_fields_into_confirmed_additions() {
        let mut old = definition(json!({"a":1}));
        old["extractor_version"] = json!("observed-http-1");
        old["limitations"]
            .as_array_mut()
            .unwrap()
            .push(json!("STRUCTURE_EXTRACTION_LIMIT"));
        let new = definition(json!({"a":"changed","extra":true}));
        let result = assess_definition(Some(&old), &new);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.path == "/extra" && f.kind == Kind::Enrichment)
        );
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.path == "/a" && f.kind == Kind::Difference)
        );
        assert!(result.categories.contains(&Kind::Insufficient));
        let mut other = new.clone();
        other["method"] = json!("POST");
        assert!(!assess_definition(Some(&new), &other).is_duplicate());
    }
}
