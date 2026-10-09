//! Application failures, independent of HTTP status codes.
use nexofolio_contracts::knowledge::KnowledgeError;
use nexofolio_curate::CompletionError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CurationError {
    #[error("接口不存在")]
    NotFound,
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
            KnowledgeError::CandidateBusy => Self::Busy,
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
        Self::Failed(error.to_string())
    }
}
