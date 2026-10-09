//! Status comes from durable field results, not the number of model replies/writes.
use super::{Curation, CurationError, jobs::Job, preparation::PreparedProject};
pub(super) async fn refresh(
    s: &Curation,
    prepared: &PreparedProject,
    job: &Job,
) -> Result<(), CurationError> {
    let mut outcomes = std::collections::BTreeMap::new();
    for facts in prepared
        .facts
        .iter()
        .filter(|f| job.snapshot().target.is_none_or(|e| e == f.summary.id))
    {
        let knowledge = s.reader.endpoint(facts.summary.id).await?;
        let inputs = prepared
            .inputs
            .get(&facts.summary.id)
            .cloned()
            .unwrap_or_default();
        let gaps = nexofolio_curate::pending_fields(facts, &knowledge, inputs.clone());
        for input in &inputs {
            let key = format!("{}/{}", facts.summary.id, input.key);
            if gaps.iter().any(|i| i.key == input.key) {
                outcomes.insert(key, 0);
                continue;
            }
            let note = knowledge.fields.iter().find(|n| {
                n.path == input.key || (n.context.location.is_none() && n.path == input.path)
            });
            let missing_values = input.observations.as_ref().is_some_and(|v| {
                v.total == 0 && !v.states.contains_key("none") && !v.states.contains_key("absent")
            });
            outcomes.insert(
                key,
                note.map_or(0, |n| {
                    if !nexofolio_curate::field_addressed(n) || missing_values {
                        2
                    } else {
                        outcome(&n.context)
                    }
                }),
            );
        }
    }
    job.update(|p| {
        p.field_outcomes = outcomes;
        counts(p);
    });
    Ok(())
}
pub(super) fn outcome(context: &nexofolio_contracts::knowledge::FieldContext) -> u8 {
    let addressed = context.completion.as_ref().is_some_and(|c| {
        c.name.state != nexofolio_contracts::knowledge::AspectState::Missing
            && c.description.state != nexofolio_contracts::knowledge::AspectState::Missing
    });
    if context.completion.is_some() && !addressed {
        return 0;
    }
    let uncertain = context.completion.as_ref().is_some_and(|c| c.uncertain());
    let unavailable = context.observations.as_ref().is_some_and(|v| {
        v.total == 0 && !v.states.contains_key("none") && !v.states.contains_key("absent")
    });
    if uncertain || unavailable { 2 } else { 1 }
}
pub(super) fn counts(p: &mut super::Progress) {
    p.fields_filled = p.field_outcomes.values().filter(|v| **v == 1).count();
    p.fields_confirmation = p.field_outcomes.values().filter(|v| **v == 2).count();
    p.fields_pending = p.field_outcomes.values().filter(|v| **v == 0).count();
    p.fields_in_scope = p.field_outcomes.len();
}
