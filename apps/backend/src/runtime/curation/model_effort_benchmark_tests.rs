//! Opt-in high/max follow-up with exact prior prompts, production parsing and no writes.
use super::model_benchmark_tests::{Recorded, digest, private_json};
use super::*;
use async_trait::async_trait;
use nexofolio_common::Secret;
use nexofolio_contracts::{
    endpoint::EndpointFacts,
    knowledge::{Command, EndpointKnowledge},
};
use nexofolio_curate::{CompletionError, FieldInput};
use nexofolio_curate_adapter::{LlmClient, ModelOptions, Thinking};
use nexofolio_knowledge::{CandidateSession, Knowledge};
use nexofolio_knowledge_adapter::PostgresKnowledge;
use nexofolio_observe::Observe;
use nexofolio_observe_adapter::PostgresObserve;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

#[derive(Clone, Deserialize)]
struct Case {
    facts: EndpointFacts,
    knowledge: EndpointKnowledge,
    inputs: Vec<FieldInput>,
}
struct ExactPrompt {
    inner: Recorded,
    expected: String,
}
#[async_trait]
impl Completions for ExactPrompt {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        if digest(&prompt) != self.expected {
            return Err(CompletionError::InvalidReply(
                "Benchmark input changed; no model request made".into(),
            ));
        }
        self.inner.complete(prompt).await
    }
}
fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[tokio::test]
#[ignore = "paid calls only with explicit cases/count/mode; SELECT-only frozen input"]
async fn exact_prompt_flash_effort_comparison() {
    let mode = std::env::var("BENCHMARK_EFFORT").unwrap();
    let thinking = match mode.as_str() {
        "high" => Thinking::High,
        "max" => Thinking::Max,
        _ => panic!("high/max only"),
    };
    let ids: Vec<String> = std::env::var("BENCHMARK_CASE_IDS")
        .unwrap()
        .split(',')
        .map(str::to_owned)
        .collect();
    assert!(!ids.is_empty() && ids.len() <= 15);
    assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), ids.len());
    assert_eq!(
        std::env::var("BENCHMARK_ALLOW_PAID").unwrap(),
        ids.len().to_string()
    );
    let directory = PathBuf::from(std::env::var("BENCHMARK_OUTPUT").unwrap());
    std::fs::create_dir(&directory).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    let source = PathBuf::from(std::env::var("BENCHMARK_SOURCE").unwrap());
    let fields = source.join("model-comparison-20261009");
    let stages = source.join("model-stages-20261009");
    let cases: Vec<Case> = serde_json::from_value(read(&fields.join("cases.json"))).unwrap();
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
    assert_eq!(candidate.state, CandidateState::Ready);
    let before = digest(&candidate);
    let frozen: recovery::FrozenInput =
        serde_json::from_value(candidate.execution_input.clone().unwrap()).unwrap();
    let reader = CandidateSession::new(&candidate, k.clone());
    let notes = reader.notes(project).await.unwrap();
    let mut ordered: Vec<_> = frozen.prepared.facts.iter().collect();
    ordered.sort_by_key(|f| (f.fields.len(), f.summary.id.to_string()));
    let selected = [
        ordered[ordered.len() / 2].clone(),
        ordered.last().unwrap().to_owned().clone(),
    ];
    private_json(
        &directory.join("manifest.json"),
        &json!({"candidate_sha256":before,"cases":ids,"mode":mode,"calls_limit":ids.len(),"concurrency":3,"retries":0,"connect_timeout_seconds":10,"timeout_seconds":150}),
    );
    let mut jobs = std::collections::VecDeque::from(ids);
    let mut workers = tokio::task::JoinSet::new();
    let mut summaries = Vec::new();
    let started = std::time::Instant::now();
    loop {
        while workers.len() < 3 {
            let Some(case_id) = jobs.pop_front() else {
                break;
            };
            let (kind, index) = case_id.split_once('-').unwrap();
            let index: usize = index.parse().unwrap();
            let is_field = kind == "field";
            assert!((is_field && index < 12) || (kind == "stage" && index < 3));
            let original = read(&if is_field {
                fields.join(format!("{index:02}-flash_low.json"))
            } else {
                stages.join(format!("{index}-flash_low.json"))
            });
            let expected = original["prompt_sha256"].as_str().unwrap().to_string();
            assert!(
                digest(&original["prompt"].as_str().unwrap()) == expected,
                "Original prompt fingerprint mismatch"
            );
            let gate = Arc::new(ExactPrompt {
                expected,
                inner: Recorded {
                    client: LlmClient::new(
                        std::env::var("NEXOFOLIO_CATALOG_MODEL_BASE_URL").unwrap(),
                        "deepseek-v4-flash".into(),
                        std::env::var("NEXOFOLIO_CATALOG_MODEL_API_KEY").unwrap(),
                    )
                    .with_concurrency(1)
                    .with_options(ModelOptions {
                        thinking,
                        json_output: true,
                    }),
                    data: Mutex::new(Value::Null),
                },
            });
            let case = is_field.then(|| cases[index].clone());
            let selected = selected.clone();
            let prepared = frozen.prepared.clone();
            let endpoints = frozen.summaries.clone();
            let notes = notes.clone();
            let directory = directory.clone();
            let mode = mode.clone();
            workers.spawn(async move {
                let client = gate.clone() as Arc<dyn Completions>;
                let mut summary =
                    json!({"case":case_id,"mode":mode,"expected_prompt_sha256":gate.expected});
                let result = if let Some(case) = &case {
                    summary["expected_fields"] = json!(case.inputs.len());
                    Describe::new(client, "deepseek-v4-flash".into())
                        .fields_batch_partial(&case.facts, &case.knowledge, &case.inputs)
                        .await
                        .map(|o| {
                            summary["missing"] = json!(o.missing.len());
                            o.commands
                        })
                } else if index < 2 {
                    let facts = &selected[index];
                    let input = nexofolio_curate::endpoint_input_from_fields(
                        facts,
                        prepared.inputs[&facts.summary.id].clone(),
                        prepared
                            .authentication
                            .get(&facts.summary.id)
                            .cloned()
                            .unwrap_or_default(),
                    );
                    Describe::new(client, "deepseek-v4-flash".into())
                        .endpoint_with_input(facts, &input, None)
                        .await
                        .map(|c| c.into_iter().collect())
                } else {
                    Organize::new(client, "deepseek-v4-flash".into())
                        .project_with_snapshot(&endpoints, &notes, &[], &[])
                        .await
                };
                let mut data = gate.inner.data.lock().unwrap().clone();
                summary["measurement"] = data["measurement"].clone();
                summary["elapsed_ms"] = data["measurement"]["elapsed_ms"]
                    .as_u64()
                    .or_else(|| data["elapsed_ms"].as_u64())
                    .map_or(Value::Null, |v| json!(v));
                summary["same_prompt"] =
                    json!(data["prompt_sha256"].as_str() == Some(&gate.expected));
                match result {
                    Ok(commands) => {
                        summary["valid"] = json!(true);
                        summary["commands"] = json!(commands.len());
                        if let Some(case) = case {
                            summary["evidence_values_preserved"] =
                                json!(commands.iter().all(|c| match c {
                                    Command::PutFieldNote {
                                        path,
                                        values,
                                        context,
                                        ..
                                    } => case.inputs.iter().find(|i| &i.key == path).is_some_and(
                                        |i| {
                                            values == &i.values
                                                && context.observations == i.observations
                                                && i.evidence
                                                    .iter()
                                                    .all(|e| context.evidence.contains(e))
                                        }
                                    ),
                                    _ => false,
                                }));
                        }
                        data["commands"] = json!(commands);
                    }
                    Err(e) => {
                        summary["valid"] = json!(false);
                        summary["error"] = json!(e.to_string());
                    }
                }
                if data.is_null() {
                    data = json!({});
                }
                data["summary"] = summary.clone();
                private_json(
                    &directory.join(format!("{case_id}-flash-{mode}.json")),
                    &data,
                );
                summary
            });
        }
        let Some(result) = workers.join_next().await else {
            break;
        };
        let summary = result.unwrap();
        println!(
            "case={} valid={} missing={} elapsed_ms={}",
            summary["case"],
            summary["valid"],
            summary["missing"],
            summary["measurement"]["elapsed_ms"]
        );
        summaries.push(summary);
    }
    assert!(
        before == digest(&k.candidate(project, id).await.unwrap()),
        "Candidate changed during comparison"
    );
    private_json(
        &directory.join("summary.json"),
        &json!({"candidate_unchanged":true,"elapsed_ms":started.elapsed().as_millis(),"results":summaries}),
    );
}
