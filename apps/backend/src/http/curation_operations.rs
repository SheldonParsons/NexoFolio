//! Compatibility projection for old identity metadata; never writes guessed history.
use nexofolio_access_contracts::ActorName;
use nexofolio_contracts::knowledge::*;
use serde::Serialize;
#[cfg(test)]
#[path = "curation_operations_tests.rs"]
mod tests;

#[derive(Serialize)]
pub(super) struct OperationView {
    #[serde(flatten)]
    pub event: CandidateOperation,
    pub name_source: &'static str,
}
pub(super) fn events(candidate: &CandidateOperationHistory) -> Vec<CandidateOperation> {
    let mut events = candidate.operations.clone();
    if !events.iter().any(|e| e.action == CandidateAction::Organize) {
        events.push(CandidateOperation {
            id: format!("legacy:{}:organize", candidate.id),
            action: CandidateAction::Organize,
            actor: CandidateActor::Unknown,
            at: candidate.created_at,
            source: OperationSource::Legacy,
        });
    }
    let action = match candidate.state {
        CandidateState::Applied => Some(CandidateAction::Apply),
        CandidateState::Discarded => Some(CandidateAction::Discard),
        _ => None,
    };
    if let Some(action) = action.filter(|action| !events.iter().any(|e| &e.action == action)) {
        let actor = candidate
            .reviewed_by
            .map(|id| CandidateActor::User {
                id,
                display_name: None,
            })
            .unwrap_or(CandidateActor::Unknown);
        events.push(CandidateOperation {
            id: format!("legacy:{}:{action:?}", candidate.id),
            action,
            actor,
            at: candidate.updated_at,
            source: OperationSource::Legacy,
        });
    }
    events.sort_by_key(|e| e.at);
    events
}
pub(super) fn resolve(events: Vec<CandidateOperation>, names: &[ActorName]) -> Vec<OperationView> {
    events
        .into_iter()
        .map(|mut event| {
            let name_source = match &mut event.actor {
                CandidateActor::User { id, display_name } if display_name.is_none() => {
                    if let Some(name) = names.iter().find(|n| n.id == *id) {
                        *display_name = Some(name.display_name.clone());
                        "profile"
                    } else {
                        "missing"
                    }
                }
                CandidateActor::Unknown => "missing",
                _ => "snapshot",
            };
            OperationView { event, name_source }
        })
        .collect()
}
