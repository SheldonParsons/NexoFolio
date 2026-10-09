//! Completeness is separate from factual confirmation: inferred words are not confirmed truth.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AspectState {
    #[default]
    Missing,
    Filled,
    Uncertain,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AspectCompletion {
    pub state: AspectState,
    pub reason: Option<String>,
}
impl AspectCompletion {
    pub fn filled() -> Self {
        Self {
            state: AspectState::Filled,
            reason: None,
        }
    }
    pub fn addressed(&self, text: Option<&str>) -> bool {
        match self.state {
            AspectState::Filled => text.is_some_and(|s| !s.trim().is_empty()),
            AspectState::Uncertain => self.reason.as_ref().is_some_and(|s| !s.trim().is_empty()),
            AspectState::Missing => false,
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldCompletion {
    pub name: AspectCompletion,
    pub description: AspectCompletion,
}
impl FieldCompletion {
    pub fn addressed(&self, name: Option<&str>, description: &str) -> bool {
        self.name.addressed(name) && self.description.addressed(Some(description))
    }
    pub fn uncertain(&self) -> bool {
        self.name.state == AspectState::Uncertain
            || self.description.state == AspectState::Uncertain
    }
}
