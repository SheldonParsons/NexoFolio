//! Generated proposals have their own lifecycle. Official readers never see them.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct KnowledgeSnapshot {
    #[serde(default, skip_serializing_if = "DocumentLifecycle::is_empty")]
    pub lifecycle: DocumentLifecycle,
    pub endpoints: Vec<EndpointId>,
    pub folders: Vec<Folder>,
    pub placements: Vec<Placement>,
    pub notes: Vec<EndpointNote>,
    pub fields: Vec<FieldNote>,
    pub links: Vec<Link>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    Running,
    Ready,
    Partial,
    Applied,
    Discarded,
    Interrupted,
}

/// Read-only navigation policies; selection never changes proposal lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateEntryKind {
    Review,
    Panel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateBatch {
    pub id: RoundId,
    pub author: Author,
    pub state: RoundState,
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub id: uuid::Uuid,
    pub project_id: ProjectId,
    pub target: Option<EndpointId>,
    pub parent: Option<uuid::Uuid>,
    pub state: CandidateState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub base_hash: String,
    pub base: KnowledgeSnapshot,
    pub view: KnowledgeSnapshot,
    pub batches: Vec<CandidateBatch>,
    pub progress: serde_json::Value,
    #[serde(default)]
    pub execution_input: Option<serde_json::Value>,
    pub applied_rounds: Vec<RoundId>,
    pub reviewed_by: Option<UserId>,
    #[serde(default)]
    pub operations: Vec<CandidateOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateSummary {
    pub id: uuid::Uuid,
    pub project_id: ProjectId,
    pub target: Option<EndpointId>,
    pub state: CandidateState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub commands: usize,
    pub progress: serde_json::Value,
}

/// Internal recovery cursor: no frozen input or generated content in the scan.
#[derive(Debug, Clone)]
pub struct CandidateRecovery {
    pub id: uuid::Uuid,
    pub project_id: ProjectId,
}
impl Candidate {
    pub fn summary(&self) -> CandidateSummary {
        let mut progress = if self.progress.is_object() {
            self.progress.clone()
        } else {
            serde_json::json!({})
        };
        // The frozen baseline is authoritative, including before the first
        // executor checkpoint and for older snapshots without a version.
        progress["lifecycle_version"] = serde_json::json!(self.base.lifecycle.version);
        CandidateSummary {
            id: self.id,
            project_id: self.project_id,
            target: self.target,
            state: self.state,
            created_at: self.created_at,
            updated_at: self.updated_at,
            commands: self.batches.iter().map(|b| b.commands.len()).sum(),
            progress,
        }
    }
}

/// Lifecycle and atomic application belong to knowledge, execution to runtime.
#[async_trait]
pub trait KnowledgeCandidates: Send + Sync {
    async fn review_candidate(
        &self,
        _project: ProjectId,
        _id: uuid::Uuid,
        _query: &CandidateReviewQuery,
    ) -> KnowledgeResult<serde_json::Value> {
        Err(KnowledgeError::Unavailable)
    }
    async fn apply_candidate_observed(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        actor: CandidateActor,
        _observer: &dyn ApplyObserver,
    ) -> KnowledgeResult<Vec<RoundId>> {
        self.apply_candidate_as(project, id, actor).await
    }
    async fn put_frozen_chunk(
        &self,
        project: ProjectId,
        candidate: uuid::Uuid,
        owner: uuid::Uuid,
        snapshot: uuid::Uuid,
        content: &str,
    ) -> KnowledgeResult<FrozenRef> {
        let _ = (project, candidate, owner, snapshot, content);
        Err(KnowledgeError::Unavailable)
    }
    async fn frozen_chunk(
        &self,
        project: ProjectId,
        candidate: uuid::Uuid,
        snapshot: uuid::Uuid,
        reference: &FrozenRef,
    ) -> KnowledgeResult<String> {
        let _ = (project, candidate, snapshot, reference);
        Err(KnowledgeError::Unavailable)
    }
    /// Backend-only scan of expired running proposals, in ascending ID order.
    async fn recovery_candidates(
        &self,
        after: Option<uuid::Uuid>,
        limit: usize,
    ) -> KnowledgeResult<Vec<CandidateRecovery>>;
    /// Recheck expired/running state under the project lock before claiming.
    /// None means no takeover, including an intervening manual stop/heartbeat.
    async fn recover_candidate(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
    ) -> KnowledgeResult<Option<Candidate>>;

    async fn freeze_candidate_values(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
        archive: &crate::field_values::FieldValueArchive,
    ) -> KnowledgeResult<()>;

    async fn create_candidate(
        &self,
        project: ProjectId,
        target: Option<EndpointId>,
        parent: Option<uuid::Uuid>,
    ) -> KnowledgeResult<Candidate> {
        self.create_candidate_as(project, target, parent, CandidateActor::Unknown)
            .await
    }
    async fn create_candidate_as(
        &self,
        project: ProjectId,
        target: Option<EndpointId>,
        parent: Option<uuid::Uuid>,
        actor: CandidateActor,
    ) -> KnowledgeResult<Candidate>;
    async fn candidate(&self, project: ProjectId, id: uuid::Uuid) -> KnowledgeResult<Candidate>;
    async fn candidate_diagnostics(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> KnowledgeResult<CandidateDiagnostics> {
        Ok(self.candidate(project, id).await?.into())
    }
    async fn candidate_operations(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> KnowledgeResult<CandidateOperationHistory>;
    /// Transactionally claim the same unfinished proposal, checking its official baseline.
    async fn resume_candidate(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
        input: serde_json::Value,
    ) -> KnowledgeResult<Candidate> {
        self.resume_candidate_as(project, id, owner, input, CandidateActor::Unknown)
            .await
    }
    async fn resume_candidate_as(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
        input: serde_json::Value,
        actor: CandidateActor,
    ) -> KnowledgeResult<Candidate>;
    /// Input is immutable once frozen. Owner fences obsolete workers after recovery.
    async fn freeze_candidate_input(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
        input: serde_json::Value,
    ) -> KnowledgeResult<()>;
    async fn cancel_candidate(&self, project: ProjectId, id: uuid::Uuid) -> KnowledgeResult<()>;
    async fn heartbeat_candidate(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
    ) -> KnowledgeResult<()>;
    async fn active_candidate(
        &self,
        project: ProjectId,
    ) -> KnowledgeResult<Option<CandidateSummary>>;
    async fn candidates(
        &self,
        project: ProjectId,
        limit: usize,
    ) -> KnowledgeResult<Vec<CandidateSummary>>;
    /// Preferred proposal and an optional selected row, from the complete project history.
    async fn entry_candidates(
        &self,
        project: ProjectId,
        kind: CandidateEntryKind,
        selected: Option<uuid::Uuid>,
    ) -> KnowledgeResult<Vec<CandidateSummary>>;
    async fn open_candidate_batch(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: Option<uuid::Uuid>,
        author: Author,
    ) -> KnowledgeResult<RoundId>;
    async fn append_candidate(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: Option<uuid::Uuid>,
        batch: RoundId,
        commands: &[Command],
    ) -> KnowledgeResult<Candidate>;
    /// Worker hot path: the immutable execution input is not returned or rewritten.
    async fn append_candidate_view(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: Option<uuid::Uuid>,
        batch: RoundId,
        proposed: &[Command],
    ) -> KnowledgeResult<KnowledgeSnapshot> {
        Ok(self
            .append_candidate(project, id, owner, batch, proposed)
            .await?
            .view)
    }
    async fn close_candidate_batch(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: Option<uuid::Uuid>,
        batch: RoundId,
        state: RoundState,
    ) -> KnowledgeResult<()>;
    async fn checkpoint_candidate(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        state: CandidateState,
        progress: serde_json::Value,
    ) -> KnowledgeResult<()>;
    async fn apply_candidate(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        reviewer: UserId,
    ) -> KnowledgeResult<Vec<RoundId>> {
        self.apply_candidate_as(
            project,
            id,
            CandidateActor::User {
                id: reviewer,
                display_name: None,
            },
        )
        .await
    }
    async fn apply_candidate_as(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        actor: CandidateActor,
    ) -> KnowledgeResult<Vec<RoundId>>;
    async fn discard_candidate(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        reviewer: UserId,
    ) -> KnowledgeResult<()> {
        self.discard_candidate_as(
            project,
            id,
            CandidateActor::User {
                id: reviewer,
                display_name: None,
            },
        )
        .await
    }
    async fn discard_candidate_as(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        actor: CandidateActor,
    ) -> KnowledgeResult<()>;
}
