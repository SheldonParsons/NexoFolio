//! Diagnostic probes: measure current behavior, without implementing the proposed classifier.
use nexofolio_contracts::{CatalogInterface, FieldRef};
use nexofolio_intake::{http_projection, http_projection_covers};
use nexofolio_knowledge::{definition_covers, extract_observed};
use nexofolio_rebuild::{field_id, snapshot_fields};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

fn observed(raw: &Value) -> Value {
    serde_json::to_value(extract_observed(raw).unwrap()).unwrap()
}
fn has_null(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(values) => values.iter().any(has_null),
        Value::Object(values) => values.values().any(has_null),
        _ => false,
    }
}
fn body(raw: &Value, value: Value) -> Value {
    let mut r = raw.clone();
    let text = value.to_string();
    r["payload"]["response"]["body"] =
        json!({"state":"complete","encoding":"text","bytes":text.len(),"content":text});
    r
}
fn pair(name: &str, a: &Value, b: &Value) -> Value {
    let pa = http_projection(a);
    let pb = http_projection(b);
    let da = observed(a);
    let db = observed(b);
    json!({"case":name,"known_projection":pa.is_some(),"incoming_projection":pb.is_some(),
        "fast_covered":pa.as_ref().zip(pb.as_ref()).is_some_and(|(x,y)|http_projection_covers(x,y)),
        "detailed_covered":definition_covers(&da,&db),"known_limitations":da["limitations"],"incoming_limitations":db["limitations"]})
}
pub fn run(input: &Value) -> Value {
    let mut heads: BTreeMap<String, Option<Value>> = BTreeMap::new();
    let mut missing = 0;
    let mut missing_with_null = 0;
    let mut predicted_duplicates = 0;
    let mut receipt_disagreements = vec![];
    let started = std::time::Instant::now();
    for r in input["records"].as_array().unwrap() {
        let projection = http_projection(&r["raw"]);
        if projection.is_none() {
            missing += 1;
            if ["request", "response"].iter().any(|side| {
                serde_json::from_str::<Value>(
                    r["raw"]["payload"][side]["body"]["content"]
                        .as_str()
                        .unwrap_or(""),
                )
                .is_ok_and(|v| has_null(&v))
            }) {
                missing_with_null += 1;
            }
        }
        let group = r["group"].as_str().unwrap();
        let duplicate = heads
            .get(group)
            .and_then(Option::as_ref)
            .zip(projection.as_ref())
            .is_some_and(|(a, b)| http_projection_covers(a, b));
        predicted_duplicates += usize::from(duplicate);
        if duplicate != (r["structure"] == "duplicate") {
            receipt_disagreements.push(r["id"].clone());
        }
        if !duplicate {
            heads.insert(group.into(), projection);
        }
    }
    let micros = started.elapsed().as_micros();
    let raw: Value = serde_json::from_str(include_str!(
        "../../../../contracts/ingestion/fixtures/http-batch.json"
    ))
    .unwrap();
    let seed = &raw["records"][0];
    let full = body(seed, json!({"items":[{"id":1}],"flag":true}));
    let empty = body(seed, json!({"items":[],"flag":true}));
    let null = body(seed, json!({"items":null,"flag":true}));
    let mut matrix = vec![
        pair(
            "values_only",
            &full,
            &body(seed, json!({"items":[{"id":2}],"flag":false})),
        ),
        pair("populated_to_empty", &full, &empty),
        pair("empty_to_populated", &empty, &full),
        pair("identical_null", &null, &null),
        pair("null_to_array", &null, &full),
        pair("array_to_null", &full, &null),
        pair("missing_sibling", &full, &body(seed, json!({"items":[]}))),
        pair(
            "mixed_empty_and_type_change",
            &full,
            &body(seed, json!({"items":[],"flag":"true"})),
        ),
        pair(
            "mixed_enrichment_and_type_change",
            &empty,
            &body(seed, json!({"items":[{"id":1}],"flag":"true"})),
        ),
        pair(
            "array_variants_reordered",
            &body(seed, json!([1, "x", true])),
            &body(seed, json!([true, "z", 2])),
        ),
        pair(
            "escaped_field_names",
            &body(seed, json!({"a/b":{"~":1}})),
            &body(seed, json!({"a/b":{"~":2}})),
        ),
    ];
    for state in ["truncated", "unreadable"] {
        let mut partial = full.clone();
        partial["payload"]["response"]["body"]["state"] = json!(state);
        matrix.push(pair(&format!("identical_{state}"), &partial, &partial));
        matrix.push(pair(&format!("complete_to_{state}"), &full, &partial));
    }
    let large = json!({"a":vec![1;10001],"z":1});
    let mut changed = large.clone();
    changed["z"] = json!("different_type");
    let big_a = body(seed, large);
    let big_b = body(seed, changed);
    matrix.push(pair("change_after_detailed_budget", &big_a, &big_b));
    let catalog: Vec<CatalogInterface> = serde_json::from_value(input["catalog"].clone()).unwrap();
    let fields: HashSet<_> = snapshot_fields(&catalog, &[])
        .into_iter()
        .map(|f| f.id)
        .collect();
    let mut missing_fields = BTreeMap::new();
    let mut facts_missing_field = 0;
    for f in input["observed_facts"].as_array().unwrap() {
        let reference: FieldRef = serde_json::from_value(f["subject"].clone()).unwrap();
        let id = field_id(&reference);
        if !fields.contains(&id) {
            facts_missing_field += 1;
            missing_fields.insert(id, serde_json::to_value(reference).unwrap());
        }
    }
    let checks = json!({
        "value_only_changes_are_covered":matrix[0]["fast_covered"]==true&&matrix[0]["detailed_covered"]==true,
        "populated_to_empty_keeps_known_structure":matrix[1]["fast_covered"]==true&&matrix[1]["detailed_covered"]==true,
        "empty_to_populated_is_not_suppressed":matrix[2]["fast_covered"]==false&&matrix[2]["detailed_covered"]==false,
        "mixed_change_not_hidden_by_empty_array":matrix[7]["fast_covered"]==false&&matrix[7]["detailed_covered"]==false,
        "variant_order_does_not_create_difference":matrix[9]["fast_covered"]==true&&matrix[9]["detailed_covered"]==true,
        "escaped_names_compare_stably":matrix[10]["fast_covered"]==true&&matrix[10]["detailed_covered"]==true,
        "source_records_have_ids":input["records"].as_array().unwrap().iter().all(|r|r["id"].is_string())
    });
    json!({"real_records":input["records"].as_array().unwrap().len(),"projection_absent":missing,"projection_absent_with_json_null":missing_with_null,
        "predicted_duplicates":predicted_duplicates,"receipt_disagreements":receipt_disagreements,"diagnostic_elapsed_us":micros,"debug_build":cfg!(debug_assertions),"comparison_algorithm":nexofolio_contracts::STRUCTURE_ALGORITHM,
        "boundary_matrix":matrix,"regression_checks":checks,"consumer_boundary":{"current_indexed_fields":fields.len(),"observed_facts_outside_current_index":facts_missing_field,"distinct_observed_fields_outside_current_index":missing_fields.len(),"examples":missing_fields.values().take(8).collect::<Vec<_>>()} })
}
