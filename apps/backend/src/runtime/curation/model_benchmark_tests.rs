//! Explicit opt-in, bounded comparison. SELECT-only input; never writes candidates.
use super::*;
use async_trait::async_trait;
use nexofolio_common::Secret;
use nexofolio_contracts::{endpoint::EndpointFacts, knowledge::EndpointKnowledge};
use nexofolio_curate::{CompletionError, FieldInput};
use nexofolio_curate_adapter::{LlmClient, ModelOptions, Thinking};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_adapter::PostgresKnowledge;
use nexofolio_observe::Observe;
use nexofolio_observe_adapter::PostgresObserve;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path, sync::Mutex, time::Duration};

#[derive(Clone, Serialize, Deserialize)]
struct Case {
    facts: EndpointFacts,
    knowledge: EndpointKnowledge,
    inputs: Vec<FieldInput>,
}
pub(super) fn private_json(path: &Path, value: &impl Serialize) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("new private benchmark artifact");
    file.write_all(&serde_json::to_vec(value).unwrap()).unwrap();
}
pub(super) fn digest(value: &impl Serialize) -> String {
    Sha256::digest(serde_json::to_vec(value).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn select(mut cases: Vec<Case>) -> Vec<Case> {
    cases.sort_by_key(|c| {
        (
            serde_json::to_vec(&c.inputs).unwrap().len(),
            c.facts.summary.id.to_string(),
            c.inputs[0].key.clone(),
        )
    });
    let mut selected = BTreeSet::new();
    // Cover model-handled headers, both body directions, source labels and relationships.
    for tag in [
        "request_header",
        "request_body",
        "response_header",
        "direct_source",
        "candidate_relation",
        "Sjb",
    ] {
        if let Some(index) = cases
            .iter()
            .position(|c| serde_json::to_string(&c.inputs).unwrap().contains(tag))
        {
            selected.insert(index);
        }
    }
    for n in 0..12 {
        if selected.len() == 12 {
            break;
        }
        selected.insert(n * (cases.len() - 1) / 11);
    }
    for index in 0..cases.len() {
        if selected.len() == 12 {
            break;
        }
        selected.insert(index);
    }
    selected.into_iter().map(|i| cases[i].clone()).collect()
}
pub(super) struct Recorded {
    pub(super) client: LlmClient,
    pub(super) data: Mutex<Value>,
}
#[async_trait]
impl Completions for Recorded {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        let started = std::time::Instant::now();
        let response = self.client.complete_measured(prompt).await;
        let record = match &response {
            Ok(r) => {
                json!({"prompt":prompt,"prompt_sha256":digest(&prompt),"reply":r.content,"measurement":r.measurement})
            }
            Err(e) => {
                json!({"prompt":prompt,"prompt_sha256":digest(&prompt),"error":e.to_string(),"elapsed_ms":started.elapsed().as_millis()})
            }
        };
        *self.data.lock().unwrap() = record;
        response
            .map(|r| r.content)
            .map_err(|e| CompletionError::InvalidReply(e.to_string()))
    }
}
#[tokio::test]
#[ignore = "36 paid calls only with BENCHMARK_ALLOW_PAID=36 and explicit read-only input"]
async fn frozen_model_comparison_without_candidate_writes() {
    assert_eq!(std::env::var("BENCHMARK_ALLOW_PAID").as_deref(), Ok("36"));
    let directory = std::path::PathBuf::from(std::env::var("BENCHMARK_OUTPUT").unwrap());
    std::fs::create_dir(&directory).expect("new benchmark output directory");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
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
    let reader = nexofolio_knowledge::CandidateSession::new(&candidate, k.clone());
    let mut pool = Vec::new();
    for facts in frozen.prepared.facts {
        let current = reader.endpoint(facts.summary.id).await.unwrap();
        let knowledge = EndpointKnowledge {
            note: current.note,
            ..Default::default()
        };
        let inputs = frozen.prepared.inputs[&facts.summary.id]
            .iter()
            .filter(|i| nexofolio_curate::standard_field_command(&facts, &knowledge, i).is_none())
            .cloned()
            .collect();
        for inputs in nexofolio_curate::field_batches(inputs, &knowledge) {
            pool.push(Case {
                facts: facts.clone(),
                knowledge: knowledge.clone(),
                inputs,
            });
        }
    }
    let pool_batches = pool.len();
    let cases = select(pool);
    assert_eq!(cases.len(), 12);
    private_json(&directory.join("cases.json"), &cases);
    private_json(
        &directory.join("manifest.json"),
        &json!({"candidate_sha256":before,"input_sha256":digest(&cases),"cases":12,"pool_batches":pool_batches,"calls":36,"concurrency":3,"retries":0}),
    );
    let variants = [
        ("pro_low", "deepseek-v4-pro", Thinking::Low),
        ("flash_low", "deepseek-v4-flash", Thinking::Low),
        ("flash_disabled", "deepseek-v4-flash", Thinking::Disabled),
    ];
    let mut jobs = std::collections::VecDeque::new();
    for (i, case) in cases.into_iter().enumerate() {
        for offset in 0..3 {
            jobs.push_back((i, case.clone(), variants[(i + offset) % 3]));
        }
    }
    let base = std::env::var("NEXOFOLIO_CATALOG_MODEL_BASE_URL").unwrap();
    let key = std::env::var("NEXOFOLIO_CATALOG_MODEL_API_KEY").unwrap();
    let mut workers = tokio::task::JoinSet::new();
    let mut summaries = Vec::new();
    let started = std::time::Instant::now();
    loop {
        while workers.len() < 3 {
            let Some((i, case, (variant, model, thinking))) = jobs.pop_front() else {
                break;
            };
            let recorded = Arc::new(Recorded {
                client: LlmClient::new(base.clone(), model.into(), key.clone())
                    .with_concurrency(1)
                    .with_options(ModelOptions {
                        thinking,
                        json_output: true,
                    }),
                data: Mutex::new(Value::Null),
            });
            let directory = directory.clone();
            workers.spawn(async move {
                let describe = Describe::new(recorded.clone() as Arc<dyn Completions>, model.into());
                let result = describe.fields_batch_partial(&case.facts, &case.knowledge, &case.inputs).await;
                let mut data = recorded.data.lock().unwrap().clone();
                let mut summary = json!({"case":i,"variant":variant,"expected":case.inputs.len(),"prompt_sha256":data["prompt_sha256"],"measurement":data["measurement"]});
                match result {
                    Ok(output) => {
                        summary["commands"] = json!(output.commands.len());
                        summary["missing"] = json!(output.missing.len());
                        summary["valid"] = json!(true);
                        let preserved = output.commands.iter().all(|command| match command {
                            nexofolio_contracts::knowledge::Command::PutFieldNote { path, values, context, .. } => {
                                case.inputs.iter().find(|i| &i.key == path).is_some_and(|i| values == &i.values && context.observations == i.observations && i.evidence.iter().all(|e| context.evidence.contains(e)))
                            }, _ => false,
                        });
                        summary["evidence_values_preserved"] = json!(preserved);
                        data["commands"] = json!(output.commands);
                    }
                    Err(e) => { summary["valid"] = json!(false); summary["error"] = json!(e.to_string()); }
                }
                data["summary"] = summary.clone();
                private_json(&directory.join(format!("{i:02}-{variant}.json")), &data);
                summary
            });
        }
        let Some(result) = workers.join_next().await else {
            break;
        };
        let summary = result.unwrap();
        println!(
            "case={} variant={} valid={} missing={} elapsed_ms={}",
            summary["case"],
            summary["variant"],
            summary["valid"],
            summary["missing"],
            summary["measurement"]["elapsed_ms"]
        );
        summaries.push(summary);
    }
    let after = k.candidate(project, id).await.unwrap();
    assert!(
        before == digest(&after),
        "candidate changed during benchmark"
    );
    private_json(
        &directory.join("summary.json"),
        &json!({"candidate_unchanged":true,"elapsed_ms":started.elapsed().as_millis(),"results":summaries}),
    );
}
