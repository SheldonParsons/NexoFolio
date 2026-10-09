//! SELECT-only verification of the stopped round; no migrations or model requests.
use super::*;
use nexofolio_common::Secret;
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_adapter::PostgresKnowledge;
use nexofolio_observe::Observe;
use nexofolio_observe_adapter::PostgresObserve;
use std::time::Duration;
#[tokio::test]
#[ignore = "requires explicit READONLY_DATABASE_URL/PROJECT_ID/CANDIDATE_ID; SELECT only"]
async fn stopped_snapshot_reuses_saved_fields_and_measures_new_plan_without_writes() {
    let url = Secret::new(std::env::var("READONLY_DATABASE_URL").unwrap());
    let project = std::env::var("READONLY_PROJECT_ID")
        .unwrap()
        .parse()
        .unwrap();
    let id = std::env::var("READONLY_CANDIDATE_ID")
        .unwrap()
        .parse()
        .unwrap();
    let k = Arc::new(Knowledge::new(
        PostgresKnowledge::new(&url, 2, Duration::from_secs(10)).unwrap(),
        Observe::new(PostgresObserve::new(&url, 2, Duration::from_secs(10)).unwrap()),
    ));
    let candidate = k.candidate(project, id).await.unwrap();
    assert_eq!(candidate.state, CandidateState::Interrupted);
    let frozen: recovery::FrozenInput =
        serde_json::from_value(candidate.execution_input.clone().unwrap()).unwrap();
    let reader = nexofolio_knowledge::CandidateSession::new(&candidate, k.clone());
    let mut scope = 0;
    let mut reused = 0;
    let mut rules = 0;
    let mut model_fields = 0;
    let mut model_batches = 0;
    let mut cold_rules = 0;
    let mut cold_model_batches = 0;
    for facts in &frozen.prepared.facts {
        let knowledge = reader.endpoint(facts.summary.id).await.unwrap();
        let inputs = frozen
            .prepared
            .inputs
            .get(&facts.summary.id)
            .unwrap()
            .clone();
        scope += inputs.len();
        let pending = nexofolio_curate::pending_fields(facts, &knowledge, inputs.clone());
        reused += inputs.len() - pending.len();
        let pending: Vec<_> = pending
            .into_iter()
            .filter(|i| {
                if nexofolio_curate::standard_field_command(facts, &knowledge, i).is_some() {
                    rules += 1;
                    false
                } else {
                    true
                }
            })
            .collect();
        model_fields += pending.len();
        model_batches += nexofolio_curate::field_batches(pending, &knowledge).len();
        let empty = nexofolio_contracts::knowledge::EndpointKnowledge {
            note: knowledge.note.clone(),
            ..Default::default()
        };
        let cold: Vec<_> = inputs
            .into_iter()
            .filter(|i| {
                if nexofolio_curate::standard_field_command(facts, &empty, i).is_some() {
                    cold_rules += 1;
                    false
                } else {
                    true
                }
            })
            .collect();
        cold_model_batches += nexofolio_curate::field_batches(cold, &empty).len();
    }
    assert_eq!(
        reused,
        candidate.view.fields.len(),
        "accepted fields must remain fresh after transport-only encoding changes"
    );
    assert_eq!(reused + rules + model_fields, scope);
    let after = k.candidate(project, id).await.unwrap();
    assert!(
        serde_json::to_value(&candidate).unwrap() == serde_json::to_value(after).unwrap(),
        "stopped candidate changed"
    );
    println!(
        "scope={scope} saved_reused={reused} rule_pending={rules} model_pending={model_fields} model_batches={model_batches} cold_rules={cold_rules} cold_model_batches={cold_model_batches}; candidate unchanged; model calls=0"
    );
}
