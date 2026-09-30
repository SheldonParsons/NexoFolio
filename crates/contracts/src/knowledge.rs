//! What a project knows about its endpoints beyond what traffic proves:
//! the catalogue tree, where each endpoint sits, names, descriptions and the
//! links between endpoints.
//!
//! Implemented by knowledge; read by curate, view and apps. Observe owns the
//! facts, this owns the words. Nothing here is derived from a single call:
//! every value was written by a person or by a curate pass, and every write
//! names its author and the round it belongs to.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{EndpointId, FolderId, LinkId, ProjectId, RoundId, UserId};
use serde::{Deserialize, Serialize};

/// Top-level folders an agent can be handed in one go, so step one of a
/// lookup never needs paging or a search term.
pub const MAX_TOP_LEVEL: usize = 12;

/// Root is depth 1. Two levels is the norm; the third exists for folders that
/// grew past [`SPLIT_THRESHOLD`].
pub const MAX_DEPTH: usize = 3;

/// Above this many endpoints, a folder earns sub-folders.
pub const SPLIT_THRESHOLD: usize = 30;

/// Who wrote something down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum Author {
    Person {
        user: UserId,
    },
    /// A curate pass. `pass` names which one, e.g. `links` or `describe`.
    Curate {
        pass: String,
        model: Option<String>,
    },
}

impl Author {
    pub fn is_person(&self) -> bool {
        matches!(self, Self::Person { .. })
    }
}

/// One folder as the tree reader returns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: FolderId,
    /// `None` at the top level.
    pub parent: Option<FolderId>,
    pub name: String,
    /// The one line an agent sees before deciding to look inside.
    pub summary: Option<String>,
    /// Among siblings, ascending.
    pub position: i32,
    /// Endpoints placed here, not counting sub-folders.
    pub endpoints: usize,
    /// Including every sub-folder.
    pub endpoints_deep: usize,
}

/// The whole tree of a project, plus the endpoints nothing has placed yet.
///
/// `unplaced` is not stored: it is every current endpoint of the project that
/// has no [`Placement`]. So an endpoint is browsable and searchable the moment
/// observe accepts it, with no write to knowledge and nothing to go wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalogue {
    pub project_id: ProjectId,
    /// Depth-first, siblings by `position`.
    pub folders: Vec<Folder>,
    pub unplaced: usize,
}

impl Catalogue {
    pub fn children_of(&self, parent: Option<FolderId>) -> impl Iterator<Item = &Folder> {
        self.folders.iter().filter(move |f| f.parent == parent)
    }

    pub fn top_level(&self) -> impl Iterator<Item = &Folder> {
        self.children_of(None)
    }
}

/// Where one endpoint sits. At most one per endpoint: the tree is a real tree,
/// or the endpoint count of a folder would stop meaning anything and an agent
/// walking the tree would meet the same endpoint again and again. Endpoints
/// that belong together across folders are joined by a [`Link`] instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub endpoint: EndpointId,
    pub folder: FolderId,
    pub author: Author,
    pub at: DateTime<Utc>,
}

/// The words for one endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointNote {
    pub endpoint: EndpointId,
    /// Human-readable, e.g. 技术咨询分页查询.
    pub name: Option<String>,
    /// One line: what it is for. This is what the catalogue pass reads, so it
    /// must stand alone without the field tree.
    pub purpose: Option<String>,
    pub author: Author,
    pub at: DateTime<Utc>,
}

/// The words for one field of one endpoint. `path` is the field path as observe
/// reports it, so notes survive as long as the field keeps its place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldNote {
    pub endpoint: EndpointId,
    pub path: String,
    pub text: String,
    /// Values seen for this field, when they read as a closed set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    pub author: Author,
    pub at: DateTime<Utc>,
}

/// How one endpoint relates to another. Found by matching real values across
/// samples, so every link carries the evidence that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub id: LinkId,
    /// The endpoint that provides the value.
    pub from: EndpointId,
    /// Field path on `from`, `None` when the whole response is meant.
    pub from_field: Option<String>,
    /// The endpoint that consumes it.
    pub to: EndpointId,
    pub to_field: Option<String>,
    pub relation: Relation,
    /// Why we believe it, in words a reader can check.
    pub evidence: String,
    pub author: Author,
    pub at: DateTime<Utc>,
}

/// Kinds of link. Each is found by a different match over real samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// `to_field` takes its values from a dictionary `from` returns. The signal
    /// is a `dictCode`-style argument whose value equals a field name elsewhere.
    Dictionary,
    /// An id in `from`'s response appears as a path parameter of `to`.
    ListToDetail,
    /// A value `from` returns is sent in `to`'s request body.
    Precondition,
    /// `to_field`'s observed values are a subset of what `from` offers.
    EnumSubset,
}

/// One run of curation, or one sitting of manual edits. Every write belongs to
/// a round, so undo has a single shape no matter who wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Round {
    pub id: RoundId,
    pub project_id: ProjectId,
    pub author: Author,
    pub started_at: DateTime<Utc>,
    /// `None` while it is still running.
    pub finished_at: Option<DateTime<Utc>>,
    pub state: RoundState,
    /// What it did, for the history list.
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundState {
    Running,
    Done,
    /// Stopped early; whatever it wrote before that stands.
    Failed,
    RolledBack,
}

/// What one write replaced. Stored as the previous value only, not a snapshot
/// of the project: a round over 47 endpoints costs tens of kilobytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    pub round: RoundId,
    pub seq: i64,
    pub target: Target,
    /// `None` when the write created something that did not exist.
    pub before: Option<serde_json::Value>,
}

/// What a revision points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "of", rename_all = "snake_case")]
pub enum Target {
    Folder { id: FolderId },
    Placement { endpoint: EndpointId },
    EndpointNote { endpoint: EndpointId },
    FieldNote { endpoint: EndpointId, path: String },
    Link { id: LinkId },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KnowledgeError {
    /// Retryable: storage is down.
    #[error("knowledge storage unavailable")]
    Unavailable,
    #[error("no such folder")]
    NoSuchFolder,
    #[error("no such round")]
    NoSuchRound,
    /// The round already finished. A finished round is what rollback replays,
    /// so nothing may be appended to it afterwards.
    #[error("round is already finished")]
    RoundClosed,
    /// The write would put a folder deeper than [`MAX_DEPTH`], or make a folder
    /// its own ancestor.
    #[error("folder tree would become invalid: {0}")]
    InvalidTree(String),
    /// The endpoint is not in the project the round belongs to.
    #[error("endpoint does not belong to this project")]
    WrongProject,
}

pub type KnowledgeResult<T> = Result<T, KnowledgeError>;

/// Paginated: `page` starts at 1, `limit` must be positive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointPage {
    pub items: Vec<EndpointId>,
    pub total: usize,
    pub page: usize,
    pub limit: usize,
}

/// Everything knowledge can be asked. Read by apps for the docs page, by curate
/// to decide what still needs work, and later by the MCP tools.
#[async_trait]
pub trait KnowledgeReader: Send + Sync {
    /// The tree plus the count of endpoints nothing has placed.
    async fn catalogue(&self, project: ProjectId) -> KnowledgeResult<Catalogue>;

    /// Endpoints in one folder, or the unplaced ones when `folder` is `None`.
    /// Page numbers start at 1. Returns empty items when page is beyond total.
    async fn folder_endpoints(
        &self,
        project: ProjectId,
        folder: Option<FolderId>,
        page: usize,
        limit: usize,
    ) -> KnowledgeResult<EndpointPage>;

    /// Everything written about one endpoint. Absent parts are `None`/empty,
    /// which is the normal state before the first curation.
    async fn endpoint(&self, endpoint: EndpointId) -> KnowledgeResult<EndpointKnowledge>;

    /// Notes for many endpoints at once, for the catalogue pass and list views.
    /// Endpoints with nothing written about them are left out.
    async fn notes(&self, project: ProjectId) -> KnowledgeResult<Vec<EndpointNote>>;

    /// Every link of a project, for diffing against a fresh pass.
    async fn links(&self, project: ProjectId) -> KnowledgeResult<Vec<Link>>;

    /// Most recent first.
    async fn rounds(&self, project: ProjectId, limit: usize) -> KnowledgeResult<Vec<Round>>;
}

/// What is known about one endpoint. Pairs with observe's `EndpointFacts`:
/// facts come from traffic, this comes from people and curation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EndpointKnowledge {
    pub note: Option<EndpointNote>,
    pub placement: Option<Placement>,
    /// Ancestors of the placement, top level first, for a breadcrumb.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<Folder>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldNote>,
    /// Links in both directions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
}

/// One change to knowledge. Every pass of curation, and every manual edit,
/// produces these and nothing else: a pass never touches storage itself.
///
/// So adding a pass means writing something that returns `Command`s. It does
/// not mean touching the store, the tree rules or the undo path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Command {
    /// Creates it when `id` is unknown, otherwise renames or moves it.
    PutFolder {
        id: FolderId,
        parent: Option<FolderId>,
        name: String,
        summary: Option<String>,
        position: i32,
    },
    /// Its endpoints become unplaced and its children move to its parent.
    /// Nothing is lost: unplaced endpoints stay browsable and searchable.
    RemoveFolder {
        id: FolderId,
    },
    /// Replaces any previous placement: an endpoint sits in one folder.
    Place {
        endpoint: EndpointId,
        folder: FolderId,
    },
    /// Back to unplaced.
    Unplace {
        endpoint: EndpointId,
    },
    /// `None` fields are left as they were, so the describe pass can write a
    /// name without clearing a purpose someone edited by hand.
    PutEndpointNote {
        endpoint: EndpointId,
        name: Option<String>,
        purpose: Option<String>,
    },
    PutFieldNote {
        endpoint: EndpointId,
        path: String,
        text: String,
        #[serde(default)]
        values: Vec<String>,
    },
    RemoveFieldNote {
        endpoint: EndpointId,
        path: String,
    },
    /// Links are identified by their endpoints, fields and relation: writing
    /// the same link twice only refreshes its evidence.
    PutLink {
        from: EndpointId,
        from_field: Option<String>,
        to: EndpointId,
        to_field: Option<String>,
        relation: Relation,
        evidence: String,
    },
    RemoveLink {
        id: LinkId,
    },
}

/// Writing to knowledge. Commands apply in order inside one round; if any is
/// rejected the whole call leaves nothing behind.
#[async_trait]
pub trait KnowledgeWriter: Send + Sync {
    /// Opens a round. Manual edits open one too, so undo has one shape only.
    async fn begin(&self, project: ProjectId, author: Author) -> KnowledgeResult<RoundId>;

    /// Applies commands, recording the previous value of everything touched.
    async fn apply(&self, round: RoundId, commands: &[Command]) -> KnowledgeResult<()>;

    /// Marks it done or failed. What a failed round wrote stands; the point of
    /// the state is that the history says so.
    async fn finish(
        &self,
        round: RoundId,
        state: RoundState,
        summary: Option<String>,
    ) -> KnowledgeResult<()>;

    /// Puts back every previous value of a round, newest write first. Whole
    /// rounds only: a single value is easier to just edit.
    async fn roll_back(&self, round: RoundId) -> KnowledgeResult<()>;
}
