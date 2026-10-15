//! Dedicated, bounded review reads; internal Candidate remains a full execution object.
use super::*;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CandidateReviewQuery {
    #[serde(default)]
    pub part: ReviewPart,
    pub version: Option<i64>,
    pub endpoint: Option<EndpointId>,
    pub section: Option<String>,
    #[serde(default)]
    pub offset: usize,
    pub key: Option<String>,
}
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPart {
    #[default]
    Head,
    Status,
    Changes,
    Endpoint,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyStage {
    Waiting,
    Validating,
    Writing,
    Committing,
    Committed,
    Failed,
    Unknown,
}
#[derive(Debug, Clone, Serialize)]
pub struct ApplyProgress {
    pub stage: ApplyStage,
    /// Commands interpreted in the uncommitted transaction, including protected no-ops.
    pub processed: usize,
    pub total: usize,
}
pub trait ApplyObserver: Send + Sync {
    fn update(&self, progress: ApplyProgress);
}
