//! Golden inputs captured from the implementation before the structural refactor.
//! These protect stored fingerprints and provenance from accidental regeneration.
use nexofolio_contracts::endpoint::{EndpointFacts, EndpointSummary};
use nexofolio_curate::{SampleField, field_input_hash, field_inputs};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Fixture {
    facts: EndpointFacts,
    signals: Vec<SampleField>,
    summaries: Vec<EndpointSummary>,
    expected_inputs: Value,
    expected_hashes: Vec<String>,
}

#[test]
fn v1_fixtures_decode_and_v2_recipe_invalidates_generated_interpretations() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/field-knowledge-v1.json")).unwrap();
    let inputs = field_inputs(&fixture.facts, &fixture.signals, &fixture.summaries);
    let mut compatible = serde_json::to_value(&inputs).unwrap();
    for input in compatible.as_array_mut().unwrap() {
        let input = input.as_object_mut().unwrap();
        input.remove("parent");
        input.remove("siblings");
        for evidence in input["evidence"].as_array_mut().unwrap() {
            evidence.as_object_mut().unwrap().remove("source");
        }
    }
    assert_eq!(compatible, fixture.expected_inputs);
    assert_ne!(
        inputs
            .iter()
            .map(|input| field_input_hash(&fixture.facts, None, input))
            .collect::<Vec<_>>(),
        fixture.expected_hashes
    );
}
