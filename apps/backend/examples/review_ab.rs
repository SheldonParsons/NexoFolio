//! Bounded paired experiment against frozen requests. No database, publication or retries.
use nexofolio_contracts::{KnowledgeSnapshot, MaintenanceReply, Secret};
use nexofolio_infrastructure::ChatMaintenanceModel;
use nexofolio_rebuild::{
    MaintenanceModel, ReviewSegment, snapshot_contains_reference, validate_segment,
};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};
#[tokio::main]
async fn main() {
    let config: Value =
        serde_json::from_reader(std::io::stdin()).expect("experiment configuration");
    let out = Path::new(config["output_dir"].as_str().unwrap());
    std::fs::create_dir_all(out).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out.join("started"))
        .expect("do not repeat paid experiment");
    let snapshot: KnowledgeSnapshot =
        serde_json::from_slice(&std::fs::read(config["snapshot"].as_str().unwrap()).unwrap())
            .unwrap();
    let key = std::fs::read_to_string(config["key_file"].as_str().unwrap()).unwrap();
    let model = ChatMaintenanceModel::with_timeout(
        config["base_url"].as_str().unwrap(),
        Secret::new(key.trim()),
        config["model"].as_str().unwrap().into(),
        false,
        Duration::from_millis(config["timeout_ms"].as_u64().unwrap()),
    )
    .unwrap();
    let cases = config["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 3, "one bounded round of three fixed cases");
    let challenger = config["challenger"].as_str().unwrap_or("thinking_off");
    assert!(["thinking_off", "thinking_low"].contains(&challenger));
    let single_variant = config["single_variant"] == true;
    let mut results = vec![];
    let trial = async {
        for (index, case) in cases.iter().enumerate() {
            let source: Value =
                serde_json::from_slice(&std::fs::read(case["file"].as_str().unwrap()).unwrap())
                    .unwrap();
            let recorded = &source["request"];
            let recorded_input: Value =
                serde_json::from_str(recorded["messages"][1]["content"].as_str().unwrap()).unwrap();
            let original = if config["current_prompt"] == true {
                let segment =
                    ReviewSegment::from_model_input(recorded_input["input"]["segment"].clone())
                        .unwrap();
                let current = nexofolio_application::review_input(snapshot.id, &segment);
                assert_eq!(
                    current["segment"], recorded_input["input"]["segment"],
                    "frozen materials must not change"
                );
                model
                    .prepare(
                        "review",
                        json!({"input":current,"previous_error":null}),
                        recorded["max_tokens"].as_u64().unwrap() as usize,
                    )
                    .unwrap()
            } else {
                recorded.clone()
            };
            let original = &original;
            assert_eq!(original["model"], config["model"]);
            assert!(
                original.get("thinking").is_none(),
                "baseline must match recorded settings"
            );
            let input: Value =
                serde_json::from_str(original["messages"][1]["content"].as_str().unwrap()).unwrap();
            let segment =
                ReviewSegment::from_model_input(input["input"]["segment"].clone()).unwrap();
            assert!(original.get("reasoning_effort").is_none());
            let modes = if single_variant {
                vec![challenger]
            } else if index % 2 == 0 {
                vec!["baseline", challenger]
            } else {
                vec![challenger, "baseline"]
            };
            for mode in modes {
                let name = format!("{}-{mode}", case["name"].as_str().unwrap());
                let mut request = original.clone();
                if mode == "thinking_off" {
                    request["thinking"] = json!({"type":"disabled"});
                }
                if mode == "thinking_low" {
                    request["thinking"] = json!({"type":"enabled"});
                    request["reasoning_effort"] = json!("low");
                }
                let mut invariant = request.clone();
                invariant.as_object_mut().unwrap().remove("thinking");
                invariant
                    .as_object_mut()
                    .unwrap()
                    .remove("reasoning_effort");
                assert_eq!(&invariant, original, "only reasoning controls may differ");
                std::fs::write(
                    out.join(format!("{name}.request.private.json")),
                    serde_json::to_vec_pretty(&request).unwrap(),
                )
                .unwrap();
                assert!(
                    serde_json::to_vec(&request).unwrap().len() <= 65536 * 60 / 100,
                    "production input budget"
                );
                let started = Instant::now();
                let answer = model.invoke_tracked(&request).await;
                let mut row = json!({"name":name,"mode":mode,"segment":segment.id,"units":segment.units.len(),"seconds":started.elapsed().as_secs_f64(),"materials_unchanged":true,"prompt_rebuilt":config["current_prompt"]==true,"retry_count":0});
                match answer {
                    Ok(answer) => {
                        std::fs::write(
                            out.join(format!("{name}.response.private.json")),
                            serde_json::to_vec_pretty(
                                &json!({"output":answer.content,"usage":answer.usage}),
                            )
                            .unwrap(),
                        )
                        .unwrap();
                        row["usage"] = answer.usage.unwrap_or(Value::Null);
                        match serde_json::from_value::<MaintenanceReply>(answer.content) {
                            Ok(MaintenanceReply::Review { review }) => {
                                let validation = validate_segment(&segment, &review);
                                row["coverage_valid"] = json!(validation.is_ok());
                                if let Err(error) = validation {
                                    row["validation_error"] = json!(format!("{error:?}"));
                                }
                                row["references_valid"] = json!(
                                    review
                                        .assessments
                                        .iter()
                                        .flat_map(|a| &a.evidence)
                                        .all(|r| snapshot_contains_reference(&snapshot, r))
                                );
                                let mut counts = std::collections::BTreeMap::<String, usize>::new();
                                for a in &review.assessments {
                                    *counts.entry(a.disposition.clone()).or_default() += 1;
                                }
                                row["dispositions"] = json!(counts);
                            }
                            _ => row["error"] = json!("REVIEW_CONTRACT_INVALID"),
                        }
                    }
                    Err(error) => row["error"] = json!(format!("{error:?}")),
                }
                results.push(row.clone());
                std::fs::write(
                    out.join("results.json"),
                    serde_json::to_vec_pretty(&results).unwrap(),
                )
                .unwrap();
                println!("{}", row);
            }
        }
    };
    let timed_out = tokio::time::timeout(Duration::from_secs(900), trial)
        .await
        .is_err();
    std::fs::write(out.join("completion.json"),serde_json::to_vec_pretty(&json!({"completed_calls":results.len(),"wall_time_limit_reached":timed_out,"production_changes":false,"candidate_generated":false})).unwrap()).unwrap();
}
