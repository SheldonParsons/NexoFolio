use crate::{DirectoryId, InterfaceId, ProjectId, RevisionId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeEvent {
    pub project_id: ProjectId,
    /// A committed project event position; not an arbitrary database sequence value.
    pub position: u64,
    pub kind: KnowledgeEventKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KnowledgeEventKind {
    InterfaceDiscovered {
        interface_id: InterfaceId,
    },
    RevisionApplied {
        interface_id: InterfaceId,
        revision: RevisionId,
    },
    ClassificationUnresolved {
        interface_id: InterfaceId,
    },
    RetrievalProblemObserved {
        description: String,
    },
    InspectionRequested,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RebuildScope {
    Project,
    Subtree { directory_id: DirectoryId },
}
