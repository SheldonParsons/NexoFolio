use super::Node;
use crate::CompletionError;
use nexofolio_common::{EndpointId, FolderId};
use nexofolio_contracts::knowledge::{Folder, MAX_DEPTH, MAX_TOP_LEVEL, Placement};
use std::collections::{HashMap, HashSet};

pub(super) fn merged_tree(
    nodes: &[Node],
    existing: &[Folder],
) -> Result<HashMap<FolderId, Node>, CompletionError> {
    let mut tree: HashMap<_, _> = existing
        .iter()
        .map(|f| {
            (
                f.id,
                Node {
                    id: f.id,
                    parent: f.parent,
                    name: f.name.clone(),
                    summary: f.summary.clone(),
                    position: f.position,
                },
            )
        })
        .collect();
    for node in nodes {
        tree.insert(node.id, node.clone());
    }
    if tree.values().filter(|n| n.parent.is_none()).count() > MAX_TOP_LEVEL {
        return Err(CompletionError::InvalidReply(
            "保留旧目录后顶层超过上限，请复用旧 ID 调整分组，未写入".into(),
        ));
    }
    let mut names = HashSet::new();
    for node in tree.values() {
        if !names.insert((node.parent, node.name.as_str())) {
            return Err(CompletionError::InvalidReply("同层目录重名，未写入".into()));
        }
        let mut seen = HashSet::new();
        let mut at = Some(node.id);
        while let Some(id) = at {
            if !seen.insert(id) || seen.len() > MAX_DEPTH {
                return Err(CompletionError::InvalidReply(
                    "目录存在循环或超过两层，未写入".into(),
                ));
            }
            at = tree
                .get(&id)
                .ok_or_else(|| CompletionError::InvalidReply("目录父级不存在，未写入".into()))?
                .parent;
        }
    }
    Ok(tree)
}

pub(super) fn nonempty_new_groups(
    tree: &HashMap<FolderId, Node>,
    existing: &[Folder],
    placements: &HashMap<EndpointId, FolderId>,
) -> Result<(), CompletionError> {
    let mut populated = HashSet::new();
    for folder in placements.values() {
        let mut at = Some(*folder);
        while let Some(id) = at {
            populated.insert(id);
            at = tree.get(&id).and_then(|n| n.parent);
        }
    }
    if tree
        .keys()
        .any(|id| !existing.iter().any(|f| f.id == *id) && !populated.contains(id))
    {
        return Err(CompletionError::InvalidReply(
            "新目录没有有效接口（人工归属不能强行移动），未写入".into(),
        ));
    }
    Ok(())
}

/// A stable URL must not silently turn into a different populated directory.
/// Empty retained directories remain addressable; splits/moves with shared
/// members are allowed. This is an identity guard, not semantic classification.
pub(super) fn stable_memberships(
    tree: &HashMap<FolderId, Node>,
    existing: &[Folder],
    before: &[Placement],
    after: &HashMap<EndpointId, FolderId>,
    scope: &HashSet<EndpointId>,
) -> Result<(), CompletionError> {
    let old: HashMap<_, _> = existing.iter().map(|f| (f.id, f.parent)).collect();
    let new: HashMap<_, _> = tree.iter().map(|(id, n)| (*id, n.parent)).collect();
    for f in existing {
        let old_members: HashSet<_> = before
            .iter()
            .filter(|p| scope.contains(&p.endpoint) && below(p.folder, f.id, &old))
            .map(|p| p.endpoint)
            .collect();
        let new_members: HashSet<_> = after
            .iter()
            .filter(|(_, folder)| below(**folder, f.id, &new))
            .map(|(endpoint, _)| *endpoint)
            .collect();
        if !old_members.is_empty()
            && !new_members.is_empty()
            && old_members.is_disjoint(&new_members)
        {
            return Err(CompletionError::InvalidReply(
                "不能用旧目录 ID 替换完全不同的接口分组，请创建新目录或保留合理成员，未写入".into(),
            ));
        }
    }
    Ok(())
}
fn below(folder: FolderId, parent: FolderId, tree: &HashMap<FolderId, Option<FolderId>>) -> bool {
    let mut at = Some(folder);
    let mut seen = HashSet::new();
    while let Some(id) = at {
        if !seen.insert(id) {
            return false;
        }
        if id == parent {
            return true;
        }
        at = tree.get(&id).copied().flatten();
    }
    false
}
