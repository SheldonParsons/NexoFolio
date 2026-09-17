//! Read private recording cases from stdin; print structural checks only, never raw bodies.
#[path = "recording_regression/stage1.rs"]
mod stage1;
#[path = "recording_regression/stage2.rs"]
mod stage2;
#[path = "recording_regression/stage3.rs"]
mod stage3;
#[path = "recording_regression/stage4.rs"]
mod stage4;

use nexofolio_evidence::{RelationSupport, eligible_relation, parameter_link_fact};
use nexofolio_knowledge::{definition_covers, extract_observed};
use serde_json::{Value, json};

fn definition(raw: &Value) -> Value {
    serde_json::to_value(extract_observed(raw).expect("valid recorded observation")).unwrap()
}

fn with_body(raw: &Value, body: Value) -> Value {
    let mut result = raw.clone();
    let content = serde_json::to_string(&body).unwrap();
    result["payload"]["response"]["body"]["bytes"] = json!(content.len());
    result["payload"]["response"]["body"]["content"] = json!(content);
    result
}

fn main() {
    let input: Value = serde_json::from_reader(std::io::stdin()).expect("JSON case envelope");
    if input["mode"] == "validate_documents" {
        let mut schema: Value = serde_json::from_str(include_str!(
            "../../../contracts/documents/responses.schema.json"
        ))
        .unwrap();
        schema["$ref"] = json!("#/$defs/ObservedDefinition");
        let validator = jsonschema::validator_for(&schema).unwrap();
        for detail in input["details"].as_array().unwrap() {
            assert!(
                validator.is_valid(&detail["definition"]),
                "served definition violates public schema"
            );
        }
        println!(
            "{}",
            json!({"validated":input["details"].as_array().unwrap().len()})
        );
        return;
    }
    if input["mode"] == "compact_shapes" {
        let started = std::time::Instant::now();
        let cases: Vec<_> = input["schemas"].as_array().unwrap().iter().map(|schema| {
            let compact = nexofolio_contracts::compact_observed_schema(schema);
            json!({"before_bytes":serde_json::to_vec(schema).unwrap().len(),"after_bytes":serde_json::to_vec(&compact).unwrap().len(),"schema":compact,"old_covers_new":nexofolio_contracts::observed_schema_covers(schema,&compact),"new_covers_old":nexofolio_contracts::observed_schema_covers(&compact,schema)})
        }).collect();
        println!(
            "{}",
            json!({"cases":cases,"elapsed_ms":started.elapsed().as_millis()})
        );
        return;
    }
    if input["mode"] == "stage4" {
        println!(
            "{}",
            serde_json::to_string_pretty(&stage4::run(&input)).unwrap()
        );
        return;
    }
    if input["mode"] == "stage3" {
        println!(
            "{}",
            serde_json::to_string_pretty(&stage3::run(&input)).unwrap()
        );
        return;
    }
    if input["mode"] == "stage2" {
        println!(
            "{}",
            serde_json::to_string_pretty(&stage2::run(&input)).unwrap()
        );
        return;
    }
    if input["mode"] == "assess" {
        let cases:Vec<_>=input["cases"].as_array().unwrap().iter().map(|case| {
            let mut incoming=extract_observed(&case["raw"]).unwrap();
            if !case["path_identity"].is_null() {
                nexofolio_knowledge::apply_observed_path(&mut incoming,&serde_json::from_value(case["path_identity"].clone()).unwrap()).unwrap();
            }
            let incoming=serde_json::to_value(incoming).unwrap();
            let result=nexofolio_knowledge::assess_definition(Some(&case["baseline"]),&incoming);
            json!({"source_id":case["source_id"],"path":incoming["path"],"assessment":result,"incoming_limitations":incoming["limitations"]})
        }).collect();
        println!("{}", serde_json::to_string_pretty(&cases).unwrap());
        return;
    }
    if input["mode"] == "stage1" {
        let result = stage1::run(&input);
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
        assert!(
            result["regression_checks"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v == true),
            "regression checks failed; inspect named results"
        );
        return;
    }
    let mut cases = Vec::new();
    for case in input["cases"].as_array().expect("cases") {
        let before = definition(&case["before_raw"]);
        let after = definition(&case["after_raw"]);
        let mut checks = vec![
            json!({"check":"initial_identity_matches_stored","passed":before["method"]==case["stored_before"]["method"]&&before["path"]==case["stored_before"]["path"]}),
            json!({"check":"incoming_identity_matches_stored","passed":after["method"]==case["stored_after"]["method"]&&after["path"]==case["stored_after"]["path"]}),
        ];
        let mut derived = Value::Null;
        match case["kind"].as_str().unwrap() {
            "array_information" => {
                let mut body: Value = serde_json::from_str(
                    case["after_raw"]["payload"]["response"]["body"]["content"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
                body["data"]["records"] = json!([]);
                let empty = definition(&with_body(&case["after_raw"], body.clone()));
                checks.push(json!({"check":"populated_to_empty_does_not_remove_known_item_structure","passed":definition_covers(&after,&empty)}));
                checks.push(json!({"check":"empty_to_populated_is_new_information","passed":!definition_covers(&before,&after)}));
                body["data"]["recording_probe_new_field"] = json!(true);
                checks.push(json!({"check":"empty_array_does_not_hide_unrelated_new_field","passed":!definition_covers(&after,&definition(&with_body(&case["after_raw"],body)))}));
                derived = json!({"provenance":"derived from recorded populated response; only records emptied, then an unrelated field added"});
            }
            "null_information" => {
                let pointer = "/response/body/observed_schema/properties/data/properties";
                checks.push(json!({"check":"unknown_null_receives_number_and_string_observations","passed":before.pointer(pointer).unwrap()["floorArea"]["type"]=="null" && after.pointer(pointer).unwrap()["floorArea"]["type"]=="number" && before.pointer(pointer).unwrap()["strategicContractCode"]["type"]=="null" && after.pointer(pointer).unwrap()["strategicContractCode"]["type"]=="string"}));
            }
            "extraction_budget" => {
                let raw: Value = serde_json::from_str(
                    case["after_raw"]["payload"]["response"]["body"]["content"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
                checks.push(json!({"check":"shared_budget_retains_previously_omitted_source_fields","passed":raw.get("msg").is_some()&&raw.get("popupType").is_some()&&after["response"]["body"]["observed_schema"]["properties"].get("msg").is_some()&&after["response"]["body"]["observed_schema"]["properties"].get("popupType").is_some()}));
                checks.push(json!({"check":"recorded_menu_now_fits_shared_budget","passed":!after["limitations"].as_array().unwrap().contains(&json!("STRUCTURE_EXTRACTION_LIMIT"))}));
                checks.push(json!({"check":"budget_gap_is_not_treated_as_duplicate_proof","passed":!definition_covers(&before,&after)}));
            }
            _ => panic!("unknown case kind"),
        }
        cases.push(json!({"kind":case["kind"],"checks":checks,"current_comparison_covers":definition_covers(&before,&after),"derived":derived}));
    }
    let relations: Vec<_> = input["relations"].as_array().unwrap().iter().map(|relation| {
        let source = serde_json::from_value(relation["subject"]["source"].clone()).unwrap();
        let target = serde_json::from_value(relation["subject"]["target"].clone()).unwrap();
        // Deliberately use the strongest current uniqueness flag with NO interaction.
        // Even this case must remain a hypothesis, never a confirmed relationship.
        let support = RelationSupport { alternative_sources:1, search_complete:true, cross_view_bridge:false, interaction_observed:false };
        let fact = parameter_link_fact(&source,&target,relation["data"]["transform"].as_str().unwrap(),&support);
        json!({"source_path":relation["subject"]["source"]["path"],"target_path":relation["subject"]["target"]["path"],"eligible_as_candidate":eligible_relation(&source,&target),"stored_verification":relation["data"]["verification"],"candidate_stays_inferred_without_interaction":fact.data["verification"]=="inferred"&&fact.data["complete"]==false&&fact.data["interaction_observed"]==false})
    }).collect();
    let ui_checks: Vec<_> = input["snapshots"].as_array().unwrap().iter().map(|snapshot| {
        let eligible = nexofolio_evidence::unambiguous_ui_targets(&snapshot["payload"]);
        json!({"event_id":snapshot["event_id"],"eligible_controls":eligible.len(),"unknown_controls_excluded":eligible.iter().all(|v|v["value"]["state"]!="unknown")})
    }).collect();
    assert!(
        ui_checks
            .iter()
            .all(|check| check["unknown_controls_excluded"] == true)
    );
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"cases":cases,"relations":relations,"ui_checks":ui_checks,"production_writes":false,"model_calls":0})
        )
        .unwrap()
    );
    assert!(
        cases.iter().all(|case| case["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["passed"] == true)),
        "recording regression check failed; see named checks"
    );
    assert!(
        relations
            .iter()
            .all(|r| r["candidate_stays_inferred_without_interaction"] == true)
    );
}
