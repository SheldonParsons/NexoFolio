use super::{Node, Proposal, identity::Identities, validate};
use crate::CompletionError;
use nexofolio_common::{EndpointId, FolderId};
use nexofolio_contracts::{
    endpoint::EndpointSummary,
    knowledge::{Command, Folder, Placement},
};
use std::collections::HashMap;

pub(crate) fn plan(
    proposal: Proposal,
    summaries: &[EndpointSummary],
    existing: &[Folder],
    placements: &[Placement],
) -> Result<Vec<Command>, CompletionError> {
    let ids = proposal.folders.iter().flat_map(|m| {
        m.existing_id
            .into_iter()
            .chain(m.children.iter().filter_map(|c| c.existing_id))
    });
    let mut identities = Identities::new(existing, ids)?;
    let mut nodes = vec![];
    let mut decisions = HashMap::new();
    for (position, module) in proposal.folders.into_iter().enumerate() {
        let members = member_ids(
            module.children.iter().flat_map(|g| g.endpoints.iter()),
            summaries,
        );
        let hint = identities.membership_hint(&members, true, placements);
        let root = identities.resolve(
            module.existing_id,
            None,
            module.name,
            module.summary,
            position,
            hint,
        )?;
        if module.children.is_empty() && !existing.iter().any(|f| f.id == root.id) {
            return Err(CompletionError::InvalidReply(
                "新业务模块必须有功能子目录，未写入".into(),
            ));
        }
        for (index, group) in module.children.into_iter().enumerate() {
            let members = member_ids(group.endpoints.iter(), summaries);
            let hint = identities.membership_hint(&members, false, placements);
            let child = identities.resolve(
                group.existing_id,
                Some(root.id),
                group.name,
                group.summary,
                index,
                hint,
            )?;
            for key in group.endpoints {
                decide(&key, Some(child.id), summaries, &mut decisions)?;
            }
            nodes.push(child);
        }
        nodes.push(root);
    }
    for entry in proposal.unplaced {
        if entry.reason.trim().is_empty() || entry.reason.chars().count() > 240 {
            return Err(CompletionError::InvalidReply(
                "待分类接口必须说明原因，未写入".into(),
            ));
        }
        decide(&entry.key, None, summaries, &mut decisions)?;
    }
    if decisions.len() != summaries.len() {
        return Err(CompletionError::InvalidReply(
            "目录建议遗漏接口，请逐项分类或明确待分类，未写入".into(),
        ));
    }
    let old: HashMap<_, _> = placements
        .iter()
        .filter(|p| summaries.iter().any(|s| s.id == p.endpoint))
        .map(|p| (p.endpoint, p))
        .collect();
    let mut effective = HashMap::new();
    for (endpoint, wanted) in &decisions {
        let current = old.get(endpoint);
        let target = if current.is_some_and(|p| p.author.is_person()) {
            current.map(|p| p.folder)
        } else {
            *wanted
        };
        if let Some(folder) = target {
            effective.insert(*endpoint, folder);
        }
    }
    let tree = validate::merged_tree(&nodes, existing)?;
    validate::nonempty_new_groups(&tree, existing, &effective)?;
    validate::stable_memberships(
        &tree,
        existing,
        placements,
        &effective,
        &summaries.iter().map(|s| s.id).collect(),
    )?;
    let mut commands = folder_commands(&nodes, existing);
    // Stable input order, so repeat runs and revision order are deterministic.
    for endpoint in summaries {
        let before = old.get(&endpoint.id).map(|p| p.folder);
        let after = effective.get(&endpoint.id).copied();
        if before != after {
            commands.push(match after {
                Some(folder) => Command::Place {
                    endpoint: endpoint.id,
                    folder,
                },
                None => Command::Unplace {
                    endpoint: endpoint.id,
                },
            });
        }
    }
    // Omitted old folders stay addressable. Curation never deletes shared IDs.
    Ok(commands)
}

fn member_ids<'a>(
    keys: impl Iterator<Item = &'a String>,
    summaries: &[EndpointSummary],
) -> Vec<EndpointId> {
    keys.filter_map(|key| {
        key.parse::<usize>()
            .ok()
            .and_then(|i| summaries.get(i))
            .map(|s| s.id)
    })
    .collect()
}

fn decide(
    key: &str,
    folder: Option<FolderId>,
    summaries: &[EndpointSummary],
    decisions: &mut HashMap<EndpointId, Option<FolderId>>,
) -> Result<(), CompletionError> {
    let index = key
        .parse::<usize>()
        .ok()
        .filter(|i| i.to_string() == key && *i < summaries.len())
        .ok_or_else(|| {
            CompletionError::InvalidReply("目录引用了不存在的接口编号，未写入".into())
        })?;
    if decisions.insert(summaries[index].id, folder).is_some() {
        return Err(CompletionError::InvalidReply(
            "同一接口被重复分类，未写入".into(),
        ));
    }
    Ok(())
}

fn folder_commands(nodes: &[Node], existing: &[Folder]) -> Vec<Command> {
    let changed: Vec<_> = nodes
        .iter()
        .filter(|n| {
            existing.iter().find(|f| f.id == n.id).is_none_or(|f| {
                f.parent != n.parent
                    || f.name != n.name
                    || f.summary != n.summary
                    || f.position != n.position
            })
        })
        .collect();
    let mut stages: Vec<_> = changed
        .iter()
        .filter_map(|n| {
            existing
                .iter()
                .find(|f| f.id == n.id)
                .filter(|f| f.parent != n.parent || f.name != n.name)
        })
        .collect();
    // Detach deepest movers first. Temporary names avoid SQL sibling-name
    // collisions on moves/swaps; the transaction never exposes these names.
    stages.sort_by_key(|f| std::cmp::Reverse(depth(f.id, existing)));
    let mut commands: Vec<_> = stages
        .iter()
        .map(|f| Command::PutFolder {
            id: f.id,
            parent: None,
            name: format!("__curation_{}", f.id),
            summary: f.summary.clone(),
            position: f.position,
        })
        .collect();
    for parent_level in [true, false] {
        commands.extend(
            changed
                .iter()
                .filter(|n| n.parent.is_none() == parent_level)
                .map(|n| Command::PutFolder {
                    id: n.id,
                    parent: n.parent,
                    name: n.name.clone(),
                    summary: n.summary.clone(),
                    position: n.position,
                }),
        );
    }
    commands
}

fn depth(id: FolderId, existing: &[Folder]) -> usize {
    let mut at = Some(id);
    let mut count = 0;
    while let Some(id) = at {
        count += 1;
        if count > existing.len() {
            break;
        }
        at = existing.iter().find(|f| f.id == id).and_then(|f| f.parent);
    }
    count
}
