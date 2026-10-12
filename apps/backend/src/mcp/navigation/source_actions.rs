//! Follow only explicit typed evidence references. No source prefetch or inferred IDs.
use super::projection::action;
use super::*;
use nexofolio_contracts::{
    endpoint::FieldLocation,
    knowledge::{EvidenceSource, FieldEvidence},
};
use std::collections::HashSet;

pub(super) fn field_source(e: &FieldEvidence) -> Option<EvidenceSource> {
    if let Some(source) = &e.source {
        return Some(source.clone());
    }
    Some(EvidenceSource {
        endpoint: Some(e.endpoint),
        example: e.example.clone()?,
        environment: e.environment,
        location: None,
        pointer: None,
        page: None,
        sequence: None,
        platform: None,
        complete: None,
    })
}
pub(super) fn reads(
    q: &Query,
    sources: &[EvidenceSource],
    summaries: &[EndpointSummary],
) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut reads = Vec::new();
    for source in sources {
        let key = (
            source.endpoint,
            source.example.clone(),
            source.environment,
            source.location,
        );
        if !seen.insert(key) {
            continue;
        }
        let mut row = json!({"endpoint_id":source.endpoint,"example_id":source.example,"environment_id":source.environment,"location":source.location});
        let Some(id) = source.endpoint else {
            row["state"] = json!("incomplete_reference");
            reads.push(row);
            continue;
        };
        if !summaries.iter().any(|s| s.id == id) {
            row["state"] = json!("endpoint_not_in_current_index");
            reads.push(row);
            continue;
        }
        if q.environment_id
            .is_some_and(|e| source.environment.is_some_and(|v| v != e))
        {
            row["state"] = json!("outside_selected_environment");
            reads.push(row);
            continue;
        }
        let mut scope = q.clone();
        scope.endpoint_id = Some(id);
        scope.environment_id = q.environment_id.or(source.environment);
        row["fallback_samples"] = action("read_interface", &scope, json!({"section":"samples"}));
        if source.example.is_empty() {
            row["state"] = json!("incomplete_reference");
            reads.push(row);
            continue;
        }
        row["state"] = json!("explicit_reference");
        row["availability"] = json!("not_prefetched");
        row["context"] = action(
            "read_source_evidence",
            &scope,
            json!({"example_id":source.example}),
        );
        let (tool, part) = match source.location {
            Some(FieldLocation::Path | FieldLocation::Query) => ("read_request_material", "url"),
            Some(FieldLocation::RequestHeader) => ("read_request_material", "headers"),
            Some(FieldLocation::RequestBody) => ("read_request_material", "body"),
            Some(FieldLocation::ResponseHeader { .. }) => ("read_response_material", "headers"),
            Some(FieldLocation::ResponseBody { .. }) => ("read_response_material", "body"),
            None => {
                row["request"] = action(
                    "read_request_material",
                    &scope,
                    json!({"example_id":source.example}),
                );
                row["response"] = action(
                    "read_response_material",
                    &scope,
                    json!({"example_id":source.example}),
                );
                reads.push(row);
                continue;
            }
        };
        row["material"] = action(
            tool,
            &scope,
            json!({"example_id":source.example,"part":part}),
        );
        reads.push(row);
    }
    reads
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use nexofolio_contracts::endpoint::EnvironmentUsage;
    #[test]
    fn references_are_explicit_scoped_and_deduplicated() {
        let project = ProjectId::new();
        let env = EnvironmentId::new();
        let endpoint = EndpointId::new();
        let other = EndpointId::new();
        let q: Query = serde_json::from_value(
            json!({"project_id":project,"environment_id":env,"endpoint_id":endpoint}),
        )
        .unwrap();
        let at = Utc.with_ymd_and_hms(2026, 10, 9, 0, 0, 0).unwrap();
        let summaries = vec![EndpointSummary {
            id: other,
            project_id: project,
            method: "GET".into(),
            path_template: "/source".into(),
            external: false,
            declared: false,
            environments: vec![EnvironmentUsage {
                environment_id: env,
                calls: 1,
                first_seen: at,
                last_seen: at,
            }],
        }];
        let source = EvidenceSource {
            endpoint: Some(other),
            example: "sample".into(),
            environment: Some(env),
            location: Some(FieldLocation::ResponseBody { status: 200 }),
            pointer: None,
            page: None,
            sequence: None,
            platform: None,
            complete: Some(true),
        };
        let mut missing = source.clone();
        missing.endpoint = None;
        let mut foreign = source.clone();
        foreign.endpoint = Some(EndpointId::new());
        let mut different = source.clone();
        different.environment = Some(EnvironmentId::new());
        let rows = reads(
            &q,
            &[source.clone(), source.clone(), missing, different, foreign],
            &summaries,
        );
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0]["material"]["tool"], "read_response_material");
        assert_eq!(rows[0]["availability"], "not_prefetched");
        assert_eq!(rows[1]["state"], "incomplete_reference");
        assert!(rows[1].get("material").is_none());
        assert_eq!(rows[2]["state"], "outside_selected_environment");
        assert!(rows[2].get("material").is_none());
        assert_eq!(rows[3]["state"], "endpoint_not_in_current_index");
        let mut unscoped = source;
        unscoped.environment = None;
        unscoped.location = None;
        let rows = reads(&q, &[unscoped], &summaries);
        assert!(rows[0].get("request").is_some());
        assert!(rows[0].get("response").is_some());
        assert_eq!(
            rows[0]["request"]["arguments"]["environment_id"],
            json!(env)
        );
    }
}
