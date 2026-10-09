#[path = "performance/baseline/mod.rs"]
#[allow(dead_code)]
mod baseline;
#[path = "stage1/mod.rs"]
#[allow(dead_code)]
mod stage1;
use nexofolio_curate::*;
use serde_json::{Value, json};

#[test]
fn indexed_project_preserves_every_scoped_relation_and_field_input() {
    let f = stage1::fixture();
    let project = ProjectEvidence::new(&f.facts, &f.signals);
    let old:Vec<_>=baseline::navigation::navigation_links(&f.facts,&f.signals).iter().map(|p|
        json!({"from":p.from,"to":p.to,"from_field":p.from_field,"to_field":p.to_field,"relation":p.relation,"context":p.context,"evidence":p.evidence})).collect();
    let new:Vec<_>=project.links().iter().map(|p|
        json!({"from":p.from,"to":p.to,"from_field":p.from_field,"to_field":p.to_field,"relation":p.relation,"context":p.context,"evidence":p.evidence})).collect();
    assert_eq!(old, new);
    let summaries: Vec<_> = f.facts.iter().map(|f| f.summary.clone()).collect();
    for facts in &f.facts {
        assert_eq!(
            serde_json::to_value(project.fields(facts)).unwrap(),
            serde_json::to_value(field_inputs(facts, &f.signals, &summaries)).unwrap()
        );
    }
}

#[test]
fn indexed_scalar_dedup_preserves_values_rows_order_and_provenance() {
    let f = stage1::fixture();
    let data: Value =
        serde_json::from_str(include_str!("fixtures/stage1-observations.json")).unwrap();
    for (index, row) in data.as_array().unwrap().iter().enumerate() {
        let facts = f
            .facts
            .iter()
            .find(|f| {
                f.summary.path_template == row["path"].as_str().unwrap()
                    && f.summary.method == row["method"].as_str().unwrap()
            })
            .unwrap();
        let example = format!("comparison-{index}");
        let old = baseline::samples::sample_fields(facts, &example, &row["observation"]);
        let new: Vec<_> = sample_fields(facts, &example, &row["observation"])
            .into_iter()
            .filter(|s| !baseline::samples::sensitive(&s.path))
            .collect();
        assert_eq!(
            serde_json::to_value(old).unwrap(),
            serde_json::to_value(new).unwrap()
        );
    }
}
