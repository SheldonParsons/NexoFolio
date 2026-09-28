//! Field facts, computed on read from fingerprints (0003 §4.5), so changing a
//! rule needs no migration.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, TimeDelta, Utc};
use nexofolio_common::EnvironmentId;
use nexofolio_contracts::endpoint::{
    Conflict, DeclaredField, EnvironmentLabel, FieldFacts, FieldLabel, FieldLocation, FieldPath,
    PathSegment, ValueType,
};

use crate::declaration::Key;
use crate::shape::Presence;
use crate::structure::Structure;
use crate::template;

/// Fewer relevant calls than this say nothing yet.
const MIN_CALLS: u64 = 5;
/// Calls after a field (or type) appeared before it counts as added.
const ADDED_CALLS: u64 = 10;
/// Calls and time without a field before it counts as removed.
const REMOVED_CALLS: u64 = 20;
const REMOVED_SPAN: TimeDelta = TimeDelta::days(3);
/// Absent calls that contradict a declared `required`.
const CONFLICT_CALLS: u64 = 5;

/// One fingerprint as the rules see it.
pub struct Seen {
    pub environment_id: EnvironmentId,
    pub structure: Structure,
    pub calls: u64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

/// One fingerprint's view of one field.
#[derive(Debug, Clone)]
pub struct Sighting {
    pub presence: Presence,
    pub types: BTreeSet<ValueType>,
    pub calls: u64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

/// Every field observed or declared, parents before children.
pub fn fields(
    path_template: &str,
    seen: &[Seen],
    declared: &BTreeMap<Key, DeclaredField>,
) -> Vec<FieldFacts> {
    let mut keys: BTreeSet<Key> = declared.keys().cloned().collect();
    for name in template::params(path_template) {
        keys.insert((
            FieldLocation::Path,
            FieldPath(vec![PathSegment::Key(name.to_owned())]),
        ));
    }
    for fingerprint in seen {
        for (location, part) in fingerprint.structure.parts() {
            keys.extend(
                part.shape
                    .paths()
                    .into_iter()
                    .map(|path| (location, FieldPath(path))),
            );
        }
    }
    keys.into_iter()
        .map(|key| field(&key, seen, declared.get(&key).cloned()))
        .collect()
}

fn field(key: &Key, seen: &[Seen], declared: Option<DeclaredField>) -> FieldFacts {
    let (location, path) = key;
    let mut by_environment: Vec<(EnvironmentId, Vec<Sighting>)> = Vec::new();
    for fingerprint in seen {
        let Some(sighting) = sighting(fingerprint, *location, &path.0) else {
            continue;
        };
        match by_environment
            .iter_mut()
            .find(|(environment, _)| *environment == fingerprint.environment_id)
        {
            Some((_, sightings)) => sightings.push(sighting),
            None => by_environment.push((fingerprint.environment_id, vec![sighting])),
        }
    }
    by_environment.sort_by_key(|(environment, _)| environment.to_string());
    let sightings = || by_environment.iter().flat_map(|(_, sightings)| sightings);
    let types: BTreeSet<ValueType> = sightings().flat_map(|s| s.types.iter().copied()).collect();
    let labels: Vec<EnvironmentLabel> = by_environment
        .iter()
        .map(|(environment_id, sightings)| EnvironmentLabel {
            environment_id: *environment_id,
            label: label(sightings),
        })
        .collect();
    let has = |wanted: fn(&FieldLabel) -> bool| labels.iter().any(|l| wanted(&l.label));
    let differs = has(|l| *l == FieldLabel::Always) && has(|l| *l == FieldLabel::Absent);
    let absent_calls = sightings()
        .filter(|s| s.presence == Presence::Absent)
        .map(|s| s.calls)
        .sum();
    FieldFacts {
        location: *location,
        path: path.clone(),
        types: types.iter().copied().collect(),
        labels,
        differs_between_environments: differs,
        conflict: declared
            .as_ref()
            .and_then(|declared| conflict(declared, &types, absent_calls)),
        declared,
    }
}

/// `None` when the fingerprint says nothing about the field: another status,
/// a missing parent, or an incomplete part that cannot prove absence.
fn sighting(seen: &Seen, location: FieldLocation, path: &[PathSegment]) -> Option<Sighting> {
    let part = seen.structure.part(location)?;
    let (presence, shape) = part.shape.at(path);
    let presence = match presence {
        Presence::Absent if !part.complete => return None,
        Presence::Unknown => return None,
        presence => presence,
    };
    Some(Sighting {
        presence,
        types: shape.map(|shape| shape.types.clone()).unwrap_or_default(),
        calls: seen.calls,
        first_seen: seen.first_seen,
        last_seen: seen.last_seen,
    })
}

fn conflict(
    declared: &DeclaredField,
    observed: &BTreeSet<ValueType>,
    absent_calls: u64,
) -> Option<Conflict> {
    if declared.required && absent_calls >= CONFLICT_CALLS {
        return Some(Conflict::RequiredButAbsent { absent_calls });
    }
    let unexpected: Vec<ValueType> = observed
        .iter()
        .filter(|kind| **kind != ValueType::Null && !declared.types.contains(kind))
        .copied()
        .collect();
    (!declared.types.is_empty() && !unexpected.is_empty()).then_some(Conflict::TypeMismatch {
        observed: unexpected,
    })
}

/// The label of one field in one environment (table in 0003 §4.5).
pub fn label(sightings: &[Sighting]) -> FieldLabel {
    let calls = |filter: &dyn Fn(&Sighting) -> bool| -> u64 {
        sightings
            .iter()
            .filter(|s| filter(s))
            .map(|s| s.calls)
            .sum()
    };
    if calls(&|_| true) < MIN_CALLS {
        return FieldLabel::Observing;
    }
    if sightings.iter().any(|s| s.presence == Presence::Mixed) {
        return FieldLabel::Optional;
    }
    let present: Vec<&Sighting> = sightings
        .iter()
        .filter(|s| s.presence == Presence::Present)
        .collect();
    let absent: Vec<&Sighting> = sightings
        .iter()
        .filter(|s| s.presence == Presence::Absent)
        .collect();
    let (Some(p), Some(a)) = (span(&present), span(&absent)) else {
        return if present.is_empty() {
            FieldLabel::Absent
        } else {
            by_type(&present)
        };
    };
    if a.1 < p.0 {
        return if total(&present) >= ADDED_CALLS {
            FieldLabel::Added { since: p.0 }
        } else {
            FieldLabel::Observing
        };
    }
    if p.1 < a.0 {
        return if total(&absent) >= REMOVED_CALLS && a.1 - a.0 >= REMOVED_SPAN {
            FieldLabel::Removed { since: a.0 }
        } else {
            FieldLabel::Observing
        };
    }
    FieldLabel::Optional
}

/// An always-present field: `Always`, or `Polymorphic` when several non-null
/// types overlap in time, or `Added` when one type replaced another.
fn by_type(present: &[&Sighting]) -> FieldLabel {
    let mut types: Vec<(DateTime<Utc>, DateTime<Utc>, u64)> = Vec::new();
    let kinds: BTreeSet<ValueType> = present
        .iter()
        .flat_map(|s| s.types.iter().copied())
        .collect();
    for kind in kinds.into_iter().filter(|kind| *kind != ValueType::Null) {
        let with: Vec<&Sighting> = present
            .iter()
            .copied()
            .filter(|s| s.types.contains(&kind))
            .collect();
        let (first, last) = span(&with).expect("the type was seen");
        types.push((first, last, total(&with)));
    }
    if types.len() < 2 {
        return FieldLabel::Always;
    }
    types.sort();
    if types.windows(2).any(|pair| pair[1].0 <= pair[0].1) {
        return FieldLabel::Polymorphic;
    }
    let (since, _, calls) = types[types.len() - 1];
    if calls >= ADDED_CALLS {
        FieldLabel::Added { since }
    } else {
        FieldLabel::Observing
    }
}

fn span(sightings: &[&Sighting]) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let first = sightings.iter().map(|s| s.first_seen).min()?;
    let last = sightings.iter().map(|s| s.last_seen).max()?;
    Some((first, last))
}

fn total(sightings: &[&Sighting]) -> u64 {
    sightings.iter().map(|s| s.calls).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(n: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000 + n * 86_400, 0).unwrap()
    }

    fn seen(presence: Presence, types: &[ValueType], calls: u64, from: i64, to: i64) -> Sighting {
        Sighting {
            presence,
            types: types.iter().copied().collect(),
            calls,
            first_seen: day(from),
            last_seen: day(to),
        }
    }

    use Presence::{Absent, Mixed, Present};
    use ValueType::{Null, Number, String};

    #[test]
    fn observing_until_five_calls() {
        assert_eq!(
            label(&[seen(Present, &[Number], 4, 0, 1)]),
            FieldLabel::Observing
        );
        assert_eq!(
            label(&[seen(Present, &[Number], 5, 0, 1)]),
            FieldLabel::Always
        );
    }

    #[test]
    fn always_absent_and_optional() {
        assert_eq!(label(&[seen(Absent, &[], 9, 0, 1)]), FieldLabel::Absent);
        let overlapping = [
            seen(Present, &[Number], 5, 0, 4),
            seen(Absent, &[], 5, 2, 6),
        ];
        assert_eq!(label(&overlapping), FieldLabel::Optional);
        assert_eq!(
            label(&[seen(Mixed, &[Number], 9, 0, 1)]),
            FieldLabel::Optional
        );
        let with_nulls = [
            seen(Present, &[Number], 5, 0, 4),
            seen(Present, &[Null], 5, 0, 4),
        ];
        assert_eq!(
            label(&with_nulls),
            FieldLabel::Always,
            "null is not a second type"
        );
    }

    #[test]
    fn added_needs_ten_calls_after() {
        let added = [
            seen(Absent, &[], 3, 0, 1),
            seen(Present, &[Number], 10, 2, 3),
        ];
        assert_eq!(label(&added), FieldLabel::Added { since: day(2) });
        let early = [
            seen(Absent, &[], 3, 0, 1),
            seen(Present, &[Number], 9, 2, 3),
        ];
        assert_eq!(label(&early), FieldLabel::Observing);
    }

    #[test]
    fn removed_needs_twenty_calls_over_three_days() {
        let removed = [
            seen(Present, &[Number], 5, 0, 1),
            seen(Absent, &[], 20, 2, 5),
        ];
        assert_eq!(label(&removed), FieldLabel::Removed { since: day(2) });
        let few = [
            seen(Present, &[Number], 5, 0, 1),
            seen(Absent, &[], 19, 2, 5),
        ];
        assert_eq!(label(&few), FieldLabel::Observing);
        let short = [
            seen(Present, &[Number], 5, 0, 1),
            seen(Absent, &[], 20, 2, 4),
        ];
        assert_eq!(label(&short), FieldLabel::Observing);
    }

    #[test]
    fn types_overlapping_are_polymorphic_else_a_change() {
        let both = [
            seen(Present, &[Number], 5, 0, 4),
            seen(Present, &[String], 5, 2, 6),
        ];
        assert_eq!(label(&both), FieldLabel::Polymorphic);
        let changed = [
            seen(Present, &[Number], 5, 0, 1),
            seen(Present, &[String], 10, 2, 3),
        ];
        assert_eq!(label(&changed), FieldLabel::Added { since: day(2) });
        let recent = [
            seen(Present, &[Number], 5, 0, 1),
            seen(Present, &[String], 9, 2, 3),
        ];
        assert_eq!(label(&recent), FieldLabel::Observing);
    }

    #[test]
    fn conflicts() {
        let required = DeclaredField {
            required: true,
            types: vec![Number],
        };
        let numbers = BTreeSet::from([Number, Null]);
        assert_eq!(conflict(&required, &numbers, 4), None);
        assert_eq!(
            conflict(&required, &numbers, 5),
            Some(Conflict::RequiredButAbsent { absent_calls: 5 })
        );
        assert_eq!(
            conflict(&required, &BTreeSet::from([String, Number]), 0),
            Some(Conflict::TypeMismatch {
                observed: vec![String]
            })
        );
        let untyped = DeclaredField {
            required: false,
            types: vec![],
        };
        assert_eq!(conflict(&untyped, &BTreeSet::from([String]), 99), None);
    }
}
