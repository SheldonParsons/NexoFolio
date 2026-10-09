//! The actual background executor over candidate ports, with a fixed model.
use super::tests::{Endpoints, fixture};
use super::*;
use async_trait::async_trait;
use nexofolio_curate::CompletionError;
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FixedModel(AtomicUsize);
#[async_trait]
impl Completions for FixedModel {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        if prompt.contains("接口列表:") {
            assert!(
                prompt.contains("用途一") && prompt.contains("用途二"),
                "directory planning must read newly generated candidate notes"
            );
            Ok(r#"{"folders":[{"name":"业务模块","summary":"业务入口","children":[{"name":"查询","summary":"查询入口","endpoints":["0","1"]}]}],"unplaced":[]}"#.into())
        } else if prompt.contains("/api/1") {
            Ok(r#"{"name":"接口一","purpose":"用途一"}"#.into())
        } else {
            Ok(r#"{"name":"接口二","purpose":"用途二"}"#.into())
        }
    }
}
#[tokio::test]
async fn full_background_run_persists_reviewable_output_without_mutating_official_readers() {
    let (template, project, _, _) = fixture(false);
    let mut facts = Vec::new();
    for summary in template.endpoints.list(project).await.unwrap() {
        facts.push(template.endpoints.get(summary.id).await.unwrap().unwrap());
    }
    let endpoints = Arc::new(Endpoints(facts.clone()));
    let knowledge = Arc::new(Knowledge::new(InMemoryKnowledge::new(), Endpoints(facts)));
    let model = Arc::new(FixedModel(AtomicUsize::new(0)));
    let completions: Arc<dyn Completions> = model.clone();
    let s = Curation::new(
        knowledge.clone(),
        knowledge.clone(),
        endpoints,
        Arc::new(Describe::new(completions.clone(), "fixture".into())),
        Arc::new(Organize::new(completions, "fixture".into())),
    )
    .with_candidates(knowledge.clone());
    s.start(project).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if knowledge
                .candidates(project, 1)
                .await
                .unwrap()
                .first()
                .is_some_and(|c| c.state == CandidateState::Ready)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let progress = s.progress(project);
    assert_eq!(progress.status, Status::Done);
    assert_eq!(progress.generated, 2);
    let id = progress.candidate_id.unwrap();
    let saved = knowledge.candidate(project, id).await.unwrap();
    assert_eq!(saved.view.notes.len(), 2);
    assert_eq!(saved.view.folders.len(), 2);
    assert_eq!(saved.view.placements.len(), 2);
    assert!(knowledge.notes(project).await.unwrap().is_empty());
    assert!(
        knowledge
            .catalogue(project)
            .await
            .unwrap()
            .folders
            .is_empty()
    );
    assert!(knowledge.rounds(project, 100).await.unwrap().is_empty());
    let mut recovered = s.clone();
    recovered.jobs = Arc::new(Jobs::default());
    let restored = recovered.saved_progress(project).await.unwrap();
    assert_eq!(restored.candidate_id, Some(id));
    assert_eq!(restored.status, Status::Done);
    assert_eq!(model.0.load(Ordering::SeqCst), 3);
    s.start(project).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let progress = s.progress(project);
            if progress.status == Status::Done && progress.candidate_id != Some(id) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let repeat = s.progress(project);
    assert_eq!(repeat.unchanged, 0);
    assert_eq!(repeat.generated, 2);
    assert!(!repeat.directory_reused);
    assert_eq!(
        model.0.load(Ordering::SeqCst),
        6,
        "a new run must not silently inherit unapplied draft results"
    );
    let repeated = knowledge
        .candidate(project, repeat.candidate_id.unwrap())
        .await
        .unwrap();
    assert!(repeated.parent.is_none());
    assert!(repeated.base.notes.is_empty());
    assert_eq!(repeated.view.notes.len(), 2);
    assert_eq!(
        knowledge.candidate(project, id).await.unwrap().view,
        saved.view
    );
    while knowledge
        .candidate(project, repeated.id)
        .await
        .unwrap()
        .state
        == CandidateState::Running
    {
        tokio::task::yield_now().await;
    }
    knowledge
        .apply_candidate(project, repeated.id, nexofolio_common::UserId::new())
        .await
        .unwrap();
    s.start(project).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while s.progress(project).status == Status::Running {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        s.progress(project).unchanged,
        2,
        "applied official notes remain eligible for reuse"
    );
    assert_eq!(s.progress(project).generated, 0);
    let official_based = knowledge
        .candidate(project, s.progress(project).candidate_id.unwrap())
        .await
        .unwrap();
    assert!(official_based.parent.is_none());
    assert_eq!(official_based.base.notes.len(), 2);
}

#[tokio::test]
async fn default_project_and_endpoint_runs_do_not_inherit_any_unapplied_state() {
    use nexofolio_contracts::knowledge::{Author, Command, FieldContext, RoundState};
    for old_state in [
        CandidateState::Ready,
        CandidateState::Partial,
        CandidateState::Interrupted,
    ] {
        for single in [false, true] {
            let (template, project, _, _) = fixture(false);
            let summaries = template.endpoints.list(project).await.unwrap();
            let mut facts = Vec::new();
            for summary in &summaries {
                facts.push(template.endpoints.get(summary.id).await.unwrap().unwrap());
            }
            let endpoint = summaries[0].id;
            let target = single.then_some(endpoint);
            let knowledge = Arc::new(Knowledge::new(InMemoryKnowledge::new(), Endpoints(facts)));
            let old = knowledge
                .create_candidate(project, target, None)
                .await
                .unwrap();
            let round = knowledge
                .open_candidate_batch(
                    project,
                    old.id,
                    None,
                    Author::Curate {
                        pass: "fixture".into(),
                        model: None,
                    },
                )
                .await
                .unwrap();
            knowledge
                .append_candidate(
                    project,
                    old.id,
                    None,
                    round,
                    &[
                        Command::PutEndpointNote {
                            endpoint,
                            name: Some("unapplied fixture".into()),
                            purpose: Some("draft purpose".into()),
                            context: Default::default(),
                        },
                        Command::PutFieldNote {
                            endpoint,
                            path: "response_body:200:fixture".into(),
                            text: "draft field".into(),
                            values: vec![],
                            context: FieldContext::default(),
                        },
                    ],
                )
                .await
                .unwrap();
            knowledge
                .close_candidate_batch(project, old.id, None, round, RoundState::Done)
                .await
                .unwrap();
            knowledge
                .checkpoint_candidate(project, old.id, old_state, serde_json::json!({}))
                .await
                .unwrap();
            let before = knowledge.candidate(project, old.id).await.unwrap();
            let s = template.with_candidates(knowledge.clone());
            let (job, _) = s
                .jobs
                .start(project, if single { 1 } else { summaries.len() });
            job.update(|p| p.target = target);
            s.stage_run(project, target, None, &summaries, &job)
                .await
                .unwrap();
            let fresh = knowledge
                .candidate(project, job.snapshot().candidate_id.unwrap())
                .await
                .unwrap();
            assert_ne!(fresh.id, old.id);
            assert!(fresh.parent.is_none());
            assert!(
                fresh.view.notes.is_empty()
                    && fresh.view.fields.is_empty()
                    && fresh.batches.is_empty()
            );
            assert_eq!(
                serde_json::to_value(before).unwrap(),
                serde_json::to_value(knowledge.candidate(project, old.id).await.unwrap()).unwrap()
            );
        }
    }
}
