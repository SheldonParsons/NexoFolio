//! Per-project joining and progress. Contains no curation rules or ports.
use super::activity::{Activity, HISTORY_LIMIT};
use chrono::{DateTime, Utc};
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_curate::Pass;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    Idle,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Progress {
    pub lifecycle_version: u64,
    pub lifecycle_stale: bool,
    pub candidate_id: Option<uuid::Uuid>,
    pub candidate_state: Option<nexofolio_contracts::knowledge::CandidateState>,
    pub attempts: usize,
    pub attempt_history: Vec<Attempt>,
    pub recovery_sweeps: usize,
    pub directory_input_hash: Option<String>,
    pub directory_reused: bool,
    pub target: Option<EndpointId>,
    pub run_id: Option<uuid::Uuid>,
    pub started_at: Option<DateTime<Utc>>,
    pub attempt_started_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub heartbeat_at: Option<DateTime<Utc>>,
    pub preparation: Option<super::preparation_progress::PreparationProgress>,
    pub resume_available: Option<bool>,
    pub recovery_blocked: bool,
    pub status: Status,
    pub stage: Option<Pass>,
    pub completed: usize,
    pub total: usize,
    pub generated: usize,
    pub unchanged: usize,
    pub fields_completed: usize,
    pub fields_updated: usize,
    pub fields_filled: usize,
    pub fields_confirmation: usize,
    pub fields_pending: usize,
    pub fields_in_scope: usize,
    pub values_archived: bool,
    #[serde(skip)]
    pub field_outcomes: std::collections::BTreeMap<String, u8>,
    #[serde(skip)]
    pub saved_field_keys: std::collections::BTreeSet<String>,
    pub fields_total: usize,
    pub fields_failed: usize,
    pub fields_reused: usize,
    pub retries: usize,
    pub queued_batches: usize,
    pub total_batches: usize,
    pub completed_batches: usize,
    pub model_calls: usize,
    pub model_ms: u64,
    pub save_ms: u64,
    pub directory_completed: bool,
    pub folders_saved: Option<usize>,
    pub activity: Vec<Activity>,
    pub active: Vec<ActiveEndpoint>,
    pub issues: Vec<Issue>,
    pub message: Option<String>,
    pub failure_code: Option<String>,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            lifecycle_version: 0,
            lifecycle_stale: false,
            candidate_id: None,
            candidate_state: None,
            attempts: 1,
            attempt_history: vec![],
            recovery_sweeps: 0,
            directory_input_hash: None,
            directory_reused: false,
            target: None,
            run_id: None,
            started_at: None,
            attempt_started_at: None,
            updated_at: None,
            heartbeat_at: None,
            preparation: None,
            resume_available: None,
            recovery_blocked: false,
            status: Status::Idle,
            stage: None,
            completed: 0,
            total: 0,
            generated: 0,
            unchanged: 0,
            fields_completed: 0,
            fields_updated: 0,
            fields_filled: 0,
            fields_confirmation: 0,
            fields_pending: 0,
            fields_in_scope: 0,
            values_archived: false,
            saved_field_keys: Default::default(),
            field_outcomes: Default::default(),
            fields_total: 0,
            fields_failed: 0,
            fields_reused: 0,
            retries: 0,
            queued_batches: 0,
            total_batches: 0,
            completed_batches: 0,
            model_calls: 0,
            model_ms: 0,
            save_ms: 0,
            directory_completed: false,
            folders_saved: None,
            activity: vec![],
            active: vec![],
            issues: vec![],
            message: None,
            failure_code: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Attempt {
    pub run_id: uuid::Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub outcome: Option<String>,
}
impl Attempt {
    pub(super) fn start(run_id: uuid::Uuid) -> Self {
        Self {
            run_id,
            started_at: Utc::now(),
            ended_at: None,
            outcome: None,
        }
    }
}

#[derive(Default)]
pub(super) struct Jobs(Mutex<HashMap<ProjectId, Arc<Job>>>);

pub(super) struct Job {
    progress: Mutex<Progress>,
    pub(super) run_id: uuid::Uuid,
    cancelled: std::sync::atomic::AtomicBool,
    cancel_signal: tokio::sync::Notify,
}

impl Job {
    // Liveness is not work progress: a heartbeat must not reset the stall clock.
    pub(super) fn heartbeat(&self) {
        self.progress.lock().expect("curate progress").heartbeat_at = Some(Utc::now());
    }
    pub(super) fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.cancel_signal.notify_waiters();
    }
    pub(super) async fn cancelled(&self) {
        let notified = self.cancel_signal.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        notified.await;
    }

    pub(super) fn record(&self, mut event: Activity) {
        self.update(|p| {
            event.id = p.activity.last().map_or(1, |last| last.id + 1);
            p.activity.push(event);
            if p.activity.len() > HISTORY_LIMIT {
                p.activity.remove(0);
            }
        });
    }
    pub fn snapshot(&self) -> Progress {
        self.progress.lock().expect("curate progress").clone()
    }
    pub(super) fn update(&self, update: impl FnOnce(&mut Progress)) {
        let mut progress = self.progress.lock().expect("curate progress");
        update(&mut progress);
        progress.updated_at = Some(Utc::now());
    }
}

impl Jobs {
    pub(super) fn forget(&self, project: ProjectId, expected: Option<uuid::Uuid>) {
        let mut jobs = self.0.lock().expect("jobs");
        if jobs
            .get(&project)
            .is_some_and(|job| Some(job.run_id) == expected)
            && let Some(job) = jobs.remove(&project)
        {
            job.cancel();
        }
    }
    pub(super) fn cancel(&self, project: ProjectId) {
        if let Some(job) = self.0.lock().expect("jobs").get(&project) {
            job.cancel();
        }
    }

    pub fn snapshot(&self, project: ProjectId) -> Progress {
        self.0
            .lock()
            .expect("curate jobs")
            .get(&project)
            .map(|job| job.snapshot())
            .unwrap_or_default()
    }

    /// A project's concurrent callers join the same run.
    pub fn start(&self, project: ProjectId, total: usize) -> (Arc<Job>, bool) {
        let mut jobs = self.0.lock().expect("curate jobs");
        if let Some(job) = jobs
            .get(&project)
            .filter(|job| job.snapshot().status == Status::Running)
        {
            return (job.clone(), false);
        }
        let run_id = uuid::Uuid::new_v4();
        let job = Arc::new(Job {
            run_id,
            cancelled: std::sync::atomic::AtomicBool::new(false),
            cancel_signal: tokio::sync::Notify::new(),
            progress: Mutex::new(Progress {
                run_id: Some(run_id),
                attempt_history: vec![Attempt::start(run_id)],
                started_at: Some(Utc::now()),
                attempt_started_at: Some(Utc::now()),
                updated_at: Some(Utc::now()),
                status: Status::Running,
                stage: Some(Pass::Describe),
                total,
                ..Progress::default()
            }),
        });
        jobs.insert(project, job.clone());
        (job, true)
    }
}

#[cfg(test)]
mod reset_tests {
    use super::*;
    #[test]
    fn reset_does_not_cancel_a_new_run_started_after_commit() {
        let jobs = Jobs::default();
        let project = ProjectId::new();
        let (old, _) = jobs.start(project, 1);
        old.update(|progress| progress.status = Status::Done);
        let (new, _) = jobs.start(project, 1);
        jobs.forget(project, Some(old.run_id));
        assert_eq!(jobs.snapshot(project).run_id, Some(new.run_id));
        jobs.forget(project, Some(new.run_id));
        assert_eq!(jobs.snapshot(project).status, Status::Idle);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ActiveEndpoint {
    #[serde(default)]
    pub task_id: u64,
    pub endpoint: EndpointId,
    pub name: Option<String>,
    pub path: String,
    pub fields: usize,
    pub completed: usize,
    pub batch: usize,
    pub batches: usize,
    pub attempt: usize,
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub state: ActiveState,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActiveState {
    #[default]
    Model,
    Saving,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Issue {
    pub endpoint: EndpointId,
    pub stage: nexofolio_curate::Pass,
    pub code: String,
    pub message: String,
    pub fields: usize,
    pub retryable: bool,
}
