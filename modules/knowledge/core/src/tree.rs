//! Tree rules that need the whole tree to check, and the sort the reader returns.

use std::collections::{HashMap, HashSet};

use nexofolio_common::FolderId;
use nexofolio_contracts::knowledge::{Folder, KnowledgeError, KnowledgeResult, MAX_DEPTH};
use nexofolio_knowledge_contracts::FolderRow;

/// Parent of every folder, for walking upwards.
pub type Parents = HashMap<FolderId, Option<FolderId>>;

pub fn parents(folders: &[FolderRow]) -> Parents {
    folders.iter().map(|f| (f.id, f.parent)).collect()
}

/// Steps from the top level down to `folder`, counting `folder` itself. A top
/// level folder is at depth 1.
///
/// `None` when the chain does not reach the top, which only happens if a cycle
/// slipped in; callers treat that as an invalid tree rather than looping.
fn depth_of(folder: FolderId, parents: &Parents) -> Option<usize> {
    let mut depth = 1;
    let mut seen = HashSet::from([folder]);
    let mut at = folder;
    while let Some(Some(parent)) = parents.get(&at).copied() {
        if !seen.insert(parent) {
            return None;
        }
        depth += 1;
        if depth > MAX_DEPTH {
            return Some(depth);
        }
        at = parent;
    }
    Some(depth)
}

/// The deepest leaf under `folder`, counting `folder` as 1.
fn height_of(folder: FolderId, children: &HashMap<Option<FolderId>, Vec<FolderId>>) -> usize {
    children
        .get(&Some(folder))
        .map(|kids| {
            1 + kids
                .iter()
                .map(|kid| height_of(*kid, children))
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(1)
}

/// Checks that putting `folder` under `parent` keeps the tree valid: the parent
/// exists, the move does not put the folder inside its own subtree, and nothing
/// ends up deeper than [`MAX_DEPTH`].
///
/// `folder` may be a new id, in which case it has no subtree yet.
pub fn check_move(
    folder: FolderId,
    parent: Option<FolderId>,
    folders: &[FolderRow],
) -> KnowledgeResult<()> {
    let Some(parent) = parent else {
        return Ok(());
    };
    if parent == folder {
        return Err(KnowledgeError::InvalidTree(
            "a folder cannot be its own parent".into(),
        ));
    }
    if !folders.iter().any(|f| f.id == parent) {
        return Err(KnowledgeError::NoSuchFolder);
    }

    let parents = parents(folders);
    // Walking up from the parent must not meet the folder itself.
    let mut at = Some(parent);
    let mut steps = 0;
    while let Some(current) = at {
        if current == folder {
            return Err(KnowledgeError::InvalidTree(
                "a folder cannot be moved inside itself".into(),
            ));
        }
        steps += 1;
        if steps > folders.len() + 1 {
            return Err(KnowledgeError::InvalidTree("the tree has a cycle".into()));
        }
        at = parents.get(&current).copied().flatten();
    }

    let parent_depth = depth_of(parent, &parents)
        .ok_or_else(|| KnowledgeError::InvalidTree("the tree has a cycle".into()))?;
    let mut children: HashMap<Option<FolderId>, Vec<FolderId>> = HashMap::new();
    for f in folders {
        children.entry(f.parent).or_default().push(f.id);
    }
    let subtree = if folders.iter().any(|f| f.id == folder) {
        height_of(folder, &children)
    } else {
        1
    };
    if parent_depth + subtree > MAX_DEPTH {
        return Err(KnowledgeError::InvalidTree(format!(
            "the catalogue is at most {MAX_DEPTH} levels deep"
        )));
    }
    Ok(())
}

/// Depth-first, siblings by position then name, with endpoint counts filled in.
///
/// `placed` counts endpoints per folder; `endpoints_deep` adds up the subtree so
/// a caller can show a top level folder's whole size without walking it.
pub fn sorted(folders: &[FolderRow], placed: &HashMap<FolderId, usize>) -> Vec<Folder> {
    let mut children: HashMap<Option<FolderId>, Vec<&FolderRow>> = HashMap::new();
    for f in folders {
        children.entry(f.parent).or_default().push(f);
    }
    for kids in children.values_mut() {
        kids.sort_by(|a, b| {
            a.position
                .cmp(&b.position)
                .then_with(|| a.name.cmp(&b.name))
        });
    }

    let mut out = Vec::with_capacity(folders.len());
    walk(None, &children, placed, &mut out);
    out
}

fn walk(
    parent: Option<FolderId>,
    children: &HashMap<Option<FolderId>, Vec<&FolderRow>>,
    placed: &HashMap<FolderId, usize>,
    out: &mut Vec<Folder>,
) {
    for row in children.get(&parent).into_iter().flatten() {
        let own = placed.get(&row.id).copied().unwrap_or(0);
        let at = out.len();
        out.push(Folder {
            id: row.id,
            parent: row.parent,
            name: row.name.clone(),
            summary: row.summary.clone(),
            position: row.position,
            endpoints: own,
            endpoints_deep: own,
        });
        walk(Some(row.id), children, placed, out);
        // Everything appended after `at` is this folder's subtree.
        let deep: usize = out[at + 1..]
            .iter()
            .filter(|f| f.parent == Some(row.id))
            .map(|f| f.endpoints_deep)
            .sum();
        out[at].endpoints_deep = own + deep;
    }
}
