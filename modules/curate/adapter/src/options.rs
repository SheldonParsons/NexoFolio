//! Explicit, optional provider controls. Defaults retain OpenAI-compatible behavior.
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Thinking {
    #[default]
    Provider,
    Low,
    High,
    Max,
    Disabled,
}
impl Thinking {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "provider" => Some(Self::Provider),
            "low" => Some(Self::Low),
            "high" => Some(Self::High),
            "max" => Some(Self::Max),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
    pub(crate) fn switch(self) -> Option<Format> {
        match self {
            Self::Provider => None,
            Self::Disabled => Some(Format { kind: "disabled" }),
            _ => Some(Format { kind: "enabled" }),
        }
    }
    pub(crate) fn effort(self) -> Option<&'static str> {
        match self {
            Self::Low => Some("low"),
            Self::High => Some("high"),
            Self::Max => Some("max"),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ModelOptions {
    pub thinking: Thinking,
    pub json_output: bool,
}
#[derive(Serialize)]
pub(crate) struct Format {
    #[serde(rename = "type")]
    pub kind: &'static str,
}
/// No prompt, completion text, reasoning text, credentials, or upstream error body.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Measurement {
    pub queue_ms: u64,
    pub elapsed_ms: u64,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub response_model: Option<String>,
    pub finish_reason: Option<String>,
}
pub struct Completion {
    pub content: String,
    pub measurement: Measurement,
}
