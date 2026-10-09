//! Actor attribution is independent of the model author of generated knowledge.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CandidateActor {
    User {
        id: UserId,
        display_name: Option<String>,
    },
    System {
        id: String,
        display_name: String,
    },
    Unknown,
}
impl CandidateActor {
    pub fn user_id(&self) -> Option<UserId> {
        match self {
            Self::User { id, .. } => Some(*id),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateAction {
    Organize,
    Resume,
    Apply,
    Discard,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OperationSource {
    #[default]
    Recorded,
    Legacy,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateOperation {
    pub id: String,
    pub action: CandidateAction,
    pub actor: CandidateActor,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub source: OperationSource,
}
#[derive(Debug, Clone)]
pub struct CandidateOperationHistory {
    pub id: uuid::Uuid,
    pub project_id: ProjectId,
    pub state: CandidateState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub reviewed_by: Option<UserId>,
    pub operations: Vec<CandidateOperation>,
}
impl Candidate {
    pub fn operation_history(&self) -> CandidateOperationHistory {
        CandidateOperationHistory {
            id: self.id,
            project_id: self.project_id,
            state: self.state,
            created_at: self.created_at,
            updated_at: self.updated_at,
            reviewed_by: self.reviewed_by,
            operations: self.operations.clone(),
        }
    }
}
impl CandidateOperation {
    pub fn new(action: CandidateAction, actor: CandidateActor, at: DateTime<Utc>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            action,
            actor,
            at,
            source: OperationSource::Recorded,
        }
    }
}
