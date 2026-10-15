//! Application failures, independent of HTTP status codes.
use nexofolio_contracts::knowledge::KnowledgeError;
use nexofolio_curate::CompletionError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CurationError {
    #[error("接口不存在")]
    NotFound,
    #[error("{0}")]
    Model(CompletionError),
    #[error("尚未提交：任务落库未得到确认，请重试；重复提交会复用已接受任务")]
    NotSubmitted,
    #[error("服务暂时不可用")]
    Unavailable,
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Failed(String),
    #[error("项目正在整理，请等待完成后再重新生成。")]
    Busy,
}
impl From<KnowledgeError> for CurationError {
    fn from(error: KnowledgeError) -> Self {
        match error {
            KnowledgeError::Unavailable => Self::Unavailable,
            KnowledgeError::NoSuchCandidate | KnowledgeError::WrongProject => Self::NotFound,
            KnowledgeError::CandidateBusy => Self::Busy,
            KnowledgeError::LifecycleStale => Self::Conflict(error.to_string()),
            KnowledgeError::FrozenInputInvalid => Self::Conflict(error.to_string()),
            KnowledgeError::CandidateStale => Self::Conflict(
                "正式知识或人工修改已偏离本轮基线，不能静默续跑；请开始新一轮整理".into(),
            ),
            KnowledgeError::CandidateClosed => {
                Self::Conflict("本轮已关闭或执行权已改变，不能补写".into())
            }
            _ => Self::Failed(error.to_string()),
        }
    }
}
impl From<CompletionError> for CurationError {
    fn from(error: CompletionError) -> Self {
        Self::Model(error)
    }
}

impl CurationError {
    pub(super) fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::Model(error) => error.code(),
            Self::NotFound => "RESOURCE_NOT_FOUND",
            Self::Unavailable => "SERVICE_UNAVAILABLE",
            Self::NotSubmitted => "CURATION_NOT_SUBMITTED",
            Self::Conflict(_) => "CANDIDATE_CONFLICT",
            Self::Failed(_) => "PASS_FAILED",
            Self::Busy => "PROJECT_BUSY",
        }
    }
}
