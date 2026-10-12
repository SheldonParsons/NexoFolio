use super::*;
use async_trait::async_trait;
use nexofolio_common::{EndpointId, FolderId, RoundId};
use nexofolio_contracts::endpoint::EndpointFacts;
use nexofolio_contracts::endpoint::{EndpointError, EndpointReader};
use nexofolio_contracts::knowledge::*;
use nexofolio_curate::{CompletionError, Completions, Describe, Organize};
use serde_json::Value;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

#[derive(Default)]
pub(super) struct Data {
    notes: Vec<EndpointNote>,
    pub(super) fields: Vec<FieldNote>,
    rounds: Vec<(RoundId, Author, RoundState)>,
    prompts: Vec<String>,
    pub(super) directory_writes: usize,
}
#[derive(Default)]
pub(super) struct Memory {
    pub(super) data: Mutex<Data>,
    saved: Notify,
}

#[async_trait]
impl KnowledgeReader for Memory {
    async fn catalogue(&self, project: ProjectId) -> KnowledgeResult<Catalogue> {
        Ok(Catalogue {
            project_id: project,
            folders: vec![],
            unplaced: 2,
        })
    }
    async fn notes(&self, _: ProjectId) -> KnowledgeResult<Vec<EndpointNote>> {
        Ok(self.data.lock().unwrap().notes.clone())
    }
    async fn folder_endpoints(
        &self,
        _: ProjectId,
        _: Option<FolderId>,
        _: usize,
        _: usize,
    ) -> KnowledgeResult<EndpointPage> {
        Err(KnowledgeError::NoSuchFolder)
    }
    async fn endpoint(&self, id: EndpointId) -> KnowledgeResult<EndpointKnowledge> {
        let data = self.data.lock().unwrap();
        Ok(EndpointKnowledge {
            note: data.notes.iter().find(|note| note.endpoint == id).cloned(),
            fields: data
                .fields
                .iter()
                .filter(|note| note.endpoint == id)
                .cloned()
                .collect(),
            ..Default::default()
        })
    }
    async fn links(&self, _: ProjectId) -> KnowledgeResult<Vec<Link>> {
        Ok(vec![])
    }
    async fn rounds(&self, _: ProjectId, _: usize) -> KnowledgeResult<Vec<Round>> {
        Ok(vec![])
    }
}
#[async_trait]
impl KnowledgeWriter for Memory {
    async fn begin(&self, _: ProjectId, author: Author) -> KnowledgeResult<RoundId> {
        let id = RoundId::new();
        self.data
            .lock()
            .unwrap()
            .rounds
            .push((id, author, RoundState::Running));
        Ok(id)
    }
    async fn apply(&self, round: RoundId, commands: &[Command]) -> KnowledgeResult<()> {
        let mut data = self.data.lock().unwrap();
        let author = data.rounds.iter().find(|r| r.0 == round).unwrap().1.clone();
        for command in commands {
            match command {
                Command::PutEndpointNote {
                    endpoint,
                    name,
                    purpose,
                    context,
                } => {
                    data.notes.retain(|note| note.endpoint != *endpoint);
                    data.notes.push(EndpointNote {
                        endpoint: *endpoint,
                        name: name.clone(),
                        purpose: purpose.clone(),
                        context: context.clone(),
                        author: author.clone(),
                        at: "2026-09-30T00:00:00Z".parse().unwrap(),
                    });
                    self.saved.notify_one();
                }
                Command::PutFolder { .. } => data.directory_writes += 1,
                Command::PutFieldNote {
                    endpoint,
                    path,
                    text,
                    values,
                    context,
                } => {
                    data.fields
                        .retain(|note| note.endpoint != *endpoint || note.path != *path);
                    data.fields.push(FieldNote {
                        endpoint: *endpoint,
                        path: path.clone(),
                        text: text.clone(),
                        values: values.clone(),
                        context: context.clone(),
                        author: author.clone(),
                        at: "2026-09-30T00:00:00Z".parse().unwrap(),
                    });
                }
                _ => {}
            }
        }
        Ok(())
    }
    async fn finish(
        &self,
        id: RoundId,
        state: RoundState,
        _: Option<String>,
    ) -> KnowledgeResult<()> {
        self.data
            .lock()
            .unwrap()
            .rounds
            .iter_mut()
            .find(|r| r.0 == id)
            .unwrap()
            .2 = state;
        Ok(())
    }
    async fn roll_back(&self, _: RoundId) -> KnowledgeResult<()> {
        Err(KnowledgeError::NoSuchRound)
    }
}

pub(super) struct Endpoints(pub(super) Vec<EndpointFacts>);
#[async_trait]
impl EndpointReader for Endpoints {
    async fn list(&self, _: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        Ok(self.0.iter().map(|f| f.summary.clone()).collect())
    }
    async fn get(&self, id: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        Ok(self.0.iter().find(|f| f.summary.id == id).cloned())
    }
    async fn example(&self, _: EndpointId, _: &str) -> Result<Option<Value>, EndpointError> {
        Ok(None)
    }
}
pub(super) struct Model {
    memory: Arc<Memory>,
    fail_second: AtomicBool,
    fail_field_second: AtomicBool,
}
#[async_trait]
impl Completions for Model {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        self.memory.data.lock().unwrap().prompts.push(prompt.into());
        if prompt.contains("字段和可核查的证据") {
            if prompt.contains("/api/2") && self.fail_field_second.load(Ordering::SeqCst) {
                return Err(CompletionError("field service unavailable".into()));
            }
            return Ok(
                r#"{"fields":[{"key":"0","display_name":"合成字段","description":"业务状态，具体含义待确认"}]}"#.into(),
            );
        }
        if prompt.contains("接口列表:") {
            assert_eq!(
                self.memory.data.lock().unwrap().notes.len(),
                2,
                "directory must wait for all descriptions"
            );
            assert!(prompt.contains("用途一") && prompt.contains("用途二"));
            return Ok(r#"{"folders":[{"name":"业务","summary":"业务接口","children":[{"name":"查询","summary":"业务查询","endpoints":["0","1"]}]}],"unplaced":[]}"#.into());
        }
        if prompt.contains("/api/2") {
            if self.fail_second.load(Ordering::SeqCst) {
                let pending = self.memory.data.lock().unwrap().notes.is_empty();
                if pending {
                    self.memory.saved.notified().await;
                }
                return Err(CompletionError("model temporarily unavailable".into()));
            }
            Ok(r#"{"name":"接口二","purpose":"用途二"}"#.into())
        } else {
            Ok(r#"{"name":"接口一","purpose":"用途一"}"#.into())
        }
    }
}

pub(super) fn fixture(fail_second: bool) -> (Curation, ProjectId, Arc<Memory>, Arc<Model>) {
    let project = ProjectId::new();
    let facts: Vec<_> = (1..=2)
        .map(|i| EndpointFacts {
            requests: vec![],
            summary: EndpointSummary {
                id: EndpointId::new(),
                project_id: project,
                method: "GET".into(),
                path_template: format!("/api/{i}"),
                declared: false,
                external: false,
                environments: vec![],
            },
            aliases: vec![],
            addresses: vec![],
            fields: vec![],
            examples: vec![],
        })
        .collect();
    let memory = Arc::new(Memory::default());
    let model = Arc::new(Model {
        memory: memory.clone(),
        fail_second: AtomicBool::new(fail_second),
        fail_field_second: AtomicBool::new(false),
    });
    let completions: Arc<dyn Completions> = model.clone();
    let state = Curation {
        knowledge: memory.clone(),
        reader: memory.clone(),
        endpoints: Arc::new(Endpoints(facts)),
        describe: Arc::new(Describe::new(completions.clone(), "test".into())),
        organize: Arc::new(Organize::new(completions, "test".into())),
        jobs: Arc::new(Jobs::default()),
        candidates: None,
        field_concurrency: 4,
        idle_budget: std::time::Duration::from_secs(900),
        description_concurrency: 2,
        frozen: None,
        resuming: false,
        actor: nexofolio_contracts::knowledge::CandidateActor::Unknown,
        audit: None,
    };
    (state, project, memory, model)
}

pub(super) async fn execute(s: &Curation, project: ProjectId) -> Progress {
    let initial = s.start(project).await.unwrap();
    assert_eq!(initial.status, Status::Running);
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        while s.progress(project).status == Status::Running {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    s.progress(project)
}

#[tokio::test]
async fn project_workflow_describes_every_endpoint_before_directory_and_reuses_completed_notes() {
    let (s, project, memory, _) = fixture(false);
    let progress = execute(&s, project).await;
    assert_eq!(progress.status, Status::Done);
    assert_eq!(
        (progress.completed, progress.generated, progress.unchanged),
        (2, 2, 0)
    );
    assert_eq!(memory.data.lock().unwrap().directory_writes, 2);
    assert!(
        memory
            .data
            .lock()
            .unwrap()
            .rounds
            .iter()
            .all(|r| r.2 == RoundState::Done)
    );
    memory.data.lock().unwrap().prompts.clear();
    let progress = execute(&s, project).await;
    assert_eq!((progress.generated, progress.unchanged), (0, 2));
    assert_eq!(memory.data.lock().unwrap().prompts.len(), 1);
}

#[tokio::test]
async fn failure_keeps_completed_notes_and_retry_only_fills_the_remainder() {
    let (s, project, memory, model) = fixture(true);
    let progress = execute(&s, project).await;
    assert_eq!(progress.status, Status::Failed);
    assert_eq!(progress.generated, 1);
    assert_eq!(memory.data.lock().unwrap().notes.len(), 1);
    assert_eq!(memory.data.lock().unwrap().directory_writes, 0);
    assert_eq!(memory.data.lock().unwrap().rounds[0].2, RoundState::Failed);
    model.fail_second.store(false, Ordering::SeqCst);
    let progress = execute(&s, project).await;
    assert_eq!(progress.status, Status::Done);
    assert_eq!((progress.generated, progress.unchanged), (1, 1));
    assert_eq!(memory.data.lock().unwrap().notes.len(), 2);
}

#[test]
fn concurrent_callers_share_one_project_run() {
    let jobs = Jobs::default();
    let project = ProjectId::new();
    let (first, created) = jobs.start(project, 47);
    assert!(created);
    let (second, created) = jobs.start(project, 47);
    assert!(!created);
    assert!(Arc::ptr_eq(&first, &second));
    let (_, created) = jobs.start(ProjectId::new(), 1);
    assert!(created);
}

#[tokio::test]
async fn field_failure_keeps_other_endpoints_and_retry_only_updates_missing_fields() {
    use nexofolio_contracts::endpoint::{
        FieldFacts, FieldLocation, FieldPath, PathSegment, ValueType,
    };
    let (mut s, project, memory, model) = fixture(false);
    let mut facts: Vec<_> = s
        .endpoints
        .list(project)
        .await
        .unwrap()
        .into_iter()
        .map(|summary| EndpointFacts {
            requests: vec![],
            summary,
            aliases: vec![],
            addresses: vec![],
            examples: vec![],
            fields: vec![FieldFacts {
                location: FieldLocation::ResponseBody { status: 200 },
                path: FieldPath(vec![PathSegment::Key("status".into())]),
                types: vec![ValueType::String],
                labels: vec![],
                differs_between_environments: false,
                declared: None,
                conflict: None,
            }],
        })
        .collect();
    facts.sort_by(|a, b| a.summary.path_template.cmp(&b.summary.path_template));
    s.endpoints = Arc::new(Endpoints(facts));
    model.fail_field_second.store(true, Ordering::SeqCst);
    let failed = execute(&s, project).await;
    assert_eq!(failed.status, Status::Failed);
    assert_eq!(failed.fields_updated, 1);
    assert_eq!(memory.data.lock().unwrap().fields.len(), 1);
    assert_eq!(failed.fields_completed, 2);
    assert_eq!(failed.fields_failed, 1);
    assert_eq!(failed.issues.len(), 1);
    assert_eq!(memory.data.lock().unwrap().directory_writes, 2);
    model.fail_field_second.store(false, Ordering::SeqCst);
    let retry = execute(&s, project).await;
    assert_eq!(retry.status, Status::Done);
    assert_eq!(retry.fields_updated, 1);
    assert_eq!(retry.fields_completed, 2);
    assert_eq!(memory.data.lock().unwrap().fields.len(), 2);
}

#[tokio::test]
async fn single_endpoint_entry_point_keeps_rounds_and_response_counts() {
    let (s, project, memory, _) = fixture(false);
    let id = s.endpoints.list(project).await.unwrap()[0].id;
    s.start_target(project, Some(id), None).await.unwrap();
    while s.progress(project).status == Status::Running {
        tokio::task::yield_now().await;
    }
    assert_eq!(memory.data.lock().unwrap().notes.len(), 1);
    assert_eq!(memory.data.lock().unwrap().rounds[0].2, RoundState::Done);
    assert_eq!(s.progress(project).status, Status::Done);
    assert_eq!(s.progress(project).total, 1);
    assert_eq!(memory.data.lock().unwrap().directory_writes, 0);
}

#[tokio::test]
async fn single_endpoint_rejects_foreign_project_before_model_or_write() {
    let (s, project, memory, _) = fixture(false);
    let id = s.endpoints.list(project).await.unwrap()[0].id;
    assert!(matches!(
        s.start_target(ProjectId::new(), Some(id), None).await,
        Err(CurationError::NotFound)
    ));
    let data = memory.data.lock().unwrap();
    assert!(data.prompts.is_empty() && data.rounds.is_empty());
}

struct PanickingRead(Vec<EndpointSummary>);
#[async_trait]
impl EndpointReader for PanickingRead {
    async fn list(&self, _: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        Ok(self.0.clone())
    }
    async fn get(&self, _: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        panic!("fixture read panic")
    }
    async fn example(&self, _: EndpointId, _: &str) -> Result<Option<Value>, EndpointError> {
        Ok(None)
    }
}
#[tokio::test]
async fn a_worker_panic_finishes_progress_instead_of_leaving_a_running_job() {
    let (mut s, project, _, _) = fixture(false);
    s.endpoints = Arc::new(PanickingRead(s.endpoints.list(project).await.unwrap()));
    let progress = execute(&s, project).await;
    assert_eq!(progress.status, Status::Failed);
    assert!(progress.message.unwrap().contains("整理被中断"));
}
