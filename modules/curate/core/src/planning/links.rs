//! Refresh generated relations through normal knowledge commands/history; protect manual links.
use crate::{LinkProposal, NAVIGATION_RECIPE};
use nexofolio_common::EndpointId;
use nexofolio_contracts::knowledge::{Command, Link};
pub fn relation_commands(
    proposals: &[LinkProposal],
    existing: &[Link],
    target: Option<EndpointId>,
) -> Vec<Command> {
    let proposals: Vec<_> = proposals
        .iter()
        .filter(|p| target.is_none_or(|id| p.to == id))
        .collect();
    let matches = |link: &Link, p: &LinkProposal| link.identity() == p.identity();
    let mut commands = Vec::new();
    for p in &proposals {
        let old = existing.iter().find(|l| matches(l, p));
        if old.is_some_and(|l| {
            l.author.is_person() || (l.context == p.context && l.evidence == p.evidence)
        }) {
            continue;
        }
        commands.push(Command::PutLink {
            from: p.from,
            from_field: Some(p.from_field.clone()),
            to: p.to,
            to_field: Some(p.to_field.clone()),
            relation: p.relation,
            evidence: p.evidence.clone(),
            context: p.context.clone(),
        });
    }
    for old in existing.iter().filter(|l| {
        !l.author.is_person()
            && matches!(
                l.context.recipe.as_deref(),
                Some("navigation-v2") | Some(NAVIGATION_RECIPE)
            )
            && target.is_none_or(|id| l.to == id)
    }) {
        if !proposals.iter().any(|p| matches(old, p)) {
            commands.push(Command::RemoveLink { id: old.id });
        }
    }
    commands
}
