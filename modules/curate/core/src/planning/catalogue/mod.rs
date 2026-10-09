//! Two-level directory proposals, identity resolution and atomic command planning.
mod identity;
mod reconcile;
mod validate;
use nexofolio_common::FolderId;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Proposal {
    pub folders: Vec<Module>,
    pub unplaced: Vec<Unplaced>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Module {
    pub existing_id: Option<FolderId>,
    pub name: String,
    pub summary: Option<String>,
    pub children: Vec<Group>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Group {
    pub existing_id: Option<FolderId>,
    pub name: String,
    pub summary: Option<String>,
    pub endpoints: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Unplaced {
    pub key: String,
    pub reason: String,
}

#[derive(Clone)]
struct Node {
    id: FolderId,
    parent: Option<FolderId>,
    name: String,
    summary: Option<String>,
    position: i32,
}

pub(crate) use reconcile::plan;
