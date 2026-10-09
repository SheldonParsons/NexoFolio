use super::Node;
use crate::CompletionError;
use nexofolio_common::{EndpointId, FolderId};
use nexofolio_contracts::knowledge::{Folder, Placement};
use std::collections::HashSet;

pub(super) struct Identities<'a> {
    pub existing: &'a [Folder],
    reserved: HashSet<FolderId>,
    used: HashSet<FolderId>,
}
impl<'a> Identities<'a> {
    pub fn new(
        existing: &'a [Folder],
        ids: impl Iterator<Item = FolderId>,
    ) -> Result<Self, CompletionError> {
        let mut reserved = HashSet::new();
        for id in ids {
            if !existing.iter().any(|f| f.id == id) || !reserved.insert(id) {
                return Err(CompletionError(
                    "目录引用了未知或重复的旧 ID，未写入".into(),
                ));
            }
        }
        Ok(Self {
            existing,
            reserved,
            used: HashSet::new(),
        })
    }

    pub fn resolve(
        &mut self,
        explicit: Option<FolderId>,
        parent: Option<FolderId>,
        name: String,
        summary: Option<String>,
        position: usize,
        membership_hint: Option<FolderId>,
    ) -> Result<Node, CompletionError> {
        let name = name.trim().to_owned();
        let summary = summary.map(|s| s.trim().to_owned());
        if name.is_empty()
            || name.chars().count() > 80
            || summary
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.chars().count() > 400)
        {
            return Err(CompletionError("目录名称或介绍为空/过长，未写入".into()));
        }
        let available: Vec<_> = self
            .existing
            .iter()
            .filter(|f| {
                !self.used.contains(&f.id) && !self.reserved.contains(&f.id) && f.name == name
            })
            .collect();
        let id = explicit
            .or_else(|| available.iter().find(|f| f.parent == parent).map(|f| f.id))
            .or(membership_hint)
            .or_else(|| (available.len() == 1).then(|| available[0].id))
            .unwrap_or_default();
        if !self.used.insert(id) {
            return Err(CompletionError("同一旧目录被重复使用，未写入".into()));
        }
        let old = self.existing.iter().find(|f| f.id == id);
        // A missing generated introduction is not a request to erase old text.
        let summary = summary.or_else(|| old.and_then(|f| f.summary.clone()));
        let protected = old.is_some_and(|f| f.author.as_ref().is_none_or(|a| a.is_person()));
        if protected
            && old.is_some_and(|f| f.parent != parent || f.name != name || f.summary != summary)
        {
            return Err(CompletionError(
                "建议修改了人工或来源不明的目录，请保留其名称、父级和介绍".into(),
            ));
        }
        if old.is_none() && summary.is_none() {
            return Err(CompletionError("新目录必须有简短介绍，未写入".into()));
        }
        Ok(Node {
            id,
            parent,
            name,
            summary,
            position: if protected {
                old.unwrap().position
            } else {
                position as i32
            },
        })
    }

    /// Exact membership supports a rename without guessing an arbitrary old
    /// folder. Root/leaf roles and generated ownership must also agree.
    pub fn membership_hint(
        &self,
        members: &[EndpointId],
        root: bool,
        placements: &[Placement],
    ) -> Option<FolderId> {
        let wanted: HashSet<_> = members.iter().copied().collect();
        if wanted.is_empty() {
            return None;
        }
        let candidates: Vec<_> = self
            .existing
            .iter()
            .filter(|f| {
                f.parent.is_none() == root
                    && f.author.as_ref().is_some_and(|a| !a.is_person())
                    && !self.used.contains(&f.id)
                    && !self.reserved.contains(&f.id)
            })
            .filter(|f| {
                let old: HashSet<_> = placements
                    .iter()
                    .filter(|p| self.below(p.folder, f.id))
                    .map(|p| p.endpoint)
                    .collect();
                old == wanted
            })
            .collect();
        (candidates.len() == 1).then(|| candidates[0].id)
    }
    fn below(&self, folder: FolderId, ancestor: FolderId) -> bool {
        let mut at = Some(folder);
        let mut guard = 0;
        while let Some(id) = at {
            if id == ancestor {
                return true;
            }
            guard += 1;
            if guard > self.existing.len() {
                return false;
            }
            at = self
                .existing
                .iter()
                .find(|f| f.id == id)
                .and_then(|f| f.parent);
        }
        false
    }
}
