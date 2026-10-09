//! Nine additional opt-in calls: two endpoint descriptions plus a complete directory proposal.
use super::model_benchmark_tests::{Recorded, digest, private_json};
use super::*;
use nexofolio_common::Secret;
use nexofolio_curate_adapter::{LlmClient, ModelOptions, Thinking};
use nexofolio_knowledge::{CandidateSession, Knowledge};
use nexofolio_knowledge_adapter::PostgresKnowledge;
use nexofolio_observe::Observe;
use nexofolio_observe_adapter::PostgresObserve;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Mutex, time::Duration};

#[tokio::test]
#[ignore = "9 paid calls only with BENCHMARK_ALLOW_PAID=9; no writes to project"]
async fn frozen_description_and_directory_comparison() {
    assert_eq!(std::env::var("BENCHMARK_ALLOW_PAID").as_deref(), Ok("9"));
    let directory = PathBuf::from(std::env::var("BENCHMARK_OUTPUT").unwrap());
    std::fs::create_dir(&directory).unwrap();
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
    let before = digest(&candidate);
    let frozen: recovery::FrozenInput =
        serde_json::from_value(candidate.execution_input.clone().unwrap()).unwrap();
    let reader = CandidateSession::new(&candidate, k.clone());
    let notes = reader.notes(project).await.unwrap();
    let mut ordered: Vec<_> = frozen.prepared.facts.iter().collect();
    ordered.sort_by_key(|f| (f.fields.len(), f.summary.id.to_string()));
    // Cold descriptions: no prior note. Median and largest endpoint by field count.
    let selected = [
        ordered[ordered.len() / 2].clone(),
        ordered.last().unwrap().to_owned().clone(),
    ];
    let mut summaries = Vec::new();
    let started = std::time::Instant::now();
    for stage in 0..3 {
        let mut workers = tokio::task::JoinSet::new();
        for (variant, model, thinking) in [
            ("pro_low", "deepseek-v4-pro", Thinking::Low),
            ("flash_low", "deepseek-v4-flash", Thinking::Low),
            ("flash_disabled", "deepseek-v4-flash", Thinking::Disabled),
        ] {
            let recorded = Arc::new(Recorded {
                client: LlmClient::new(
                    std::env::var("NEXOFOLIO_CATALOG_MODEL_BASE_URL").unwrap(),
                    model.into(),
                    std::env::var("NEXOFOLIO_CATALOG_MODEL_API_KEY").unwrap(),
                )
                .with_concurrency(1)
                .with_options(ModelOptions {
                    thinking,
                    json_output: true,
                }),
                data: Mutex::new(Value::Null),
            });
            let selected = selected.clone();
            let prepared = frozen.prepared.clone();
            let endpoints = frozen.summaries.clone();
            let notes = notes.clone();
            let directory = directory.clone();
            workers.spawn(async move {
                let client = recorded.clone() as Arc<dyn Completions>;
                let result = if stage < 2 {
                    let facts = &selected[stage];
                    let input = nexofolio_curate::endpoint_input_from_fields(facts,prepared.inputs[&facts.summary.id].clone(),prepared.authentication.get(&facts.summary.id).cloned().unwrap_or_default());
                    Describe::new(client,model.into()).endpoint_with_input(facts,&input,None).await.map(|c| c.into_iter().collect::<Vec<_>>())
                } else {
                    // No new proposal inherits an unapplied directory.
                    Organize::new(client,model.into()).project_with_snapshot(&endpoints,&notes,&[],&[]).await
                };
                let mut data = recorded.data.lock().unwrap().clone();
                let mut summary=json!({"stage":stage,"variant":variant,"prompt_sha256":data["prompt_sha256"],"measurement":data["measurement"]});
                match result {
                    Ok(commands)=> {
                        summary["valid"]=json!(true);summary["commands"]=json!(commands.len());
                        data["commands"]=json!(commands);
                    },
                    Err(e)=> {summary["valid"]=json!(false);summary["error"]=json!(e.to_string());}
                }
                data["summary"]=summary.clone();private_json(&directory.join(format!("{stage}-{variant}.json")),&data);summary
            });
        }
        while let Some(r) = workers.join_next().await {
            summaries.push(r.unwrap());
        }
        println!("stage={stage} complete");
    }
    assert!(
        before == digest(&k.candidate(project, id).await.unwrap()),
        "candidate changed"
    );
    private_json(
        &directory.join("summary.json"),
        &json!({"candidate_unchanged":true,"elapsed_ms":started.elapsed().as_millis(),"results":summaries}),
    );
}
