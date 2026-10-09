//! Choose a single target environment before expanding its upstream graph.
use nexofolio_common::EnvironmentId;
use nexofolio_contracts::{endpoint::EndpointFacts, knowledge::EndpointKnowledge};

/// Omission is safe only when the target has zero or one known environments.
/// Multiple environments need an explicit choice; they must never be mixed.
pub fn trace_environment(
    facts: &EndpointFacts,
    knowledge: Option<&EndpointKnowledge>,
    requested: Option<EnvironmentId>,
) -> Result<Option<EnvironmentId>, Vec<EnvironmentId>> {
    let mut environments: Vec<_> = facts
        .summary
        .environments
        .iter()
        .map(|e| e.environment_id)
        .collect();
    if let Some(k) = knowledge {
        environments.extend(
            k.note
                .iter()
                .flat_map(|n| &n.context.authentication)
                .filter_map(|a| a.source.environment),
        );
        environments.extend(
            k.links
                .iter()
                .filter(|l| l.to == facts.summary.id)
                .filter_map(|l| l.context.environment),
        );
    }
    environments.sort_by_key(|env| env.to_string());
    environments.dedup();
    if let Some(env) = requested {
        if environments.is_empty() || environments.contains(&env) {
            return Ok(Some(env));
        }
        return Err(environments.into_iter().collect());
    }
    if environments.len() > 1 {
        Err(environments.into_iter().collect())
    } else {
        Ok(environments.into_iter().next())
    }
}
