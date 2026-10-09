//! A missing answer is retryable; an explicit, reasoned uncertainty is reviewable.
use super::fields::Description;
use nexofolio_contracts::knowledge::{AspectCompletion, AspectState, FieldCompletion, FieldNote};
pub(super) struct Resolved {
    pub name: Option<String>,
    pub text: String,
    pub completion: FieldCompletion,
    pub preserve_description: bool,
}
fn clean(value: Option<&str>, limit: usize) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(limit).collect())
}
fn aspect(text: Option<&str>, reason: Option<&str>) -> AspectCompletion {
    if let Some(reason) = clean(reason, 200) {
        return AspectCompletion {
            state: AspectState::Uncertain,
            reason: Some(reason),
        };
    }
    if text.is_some_and(|s| {
        !s.trim().is_empty()
            && !matches!(
                s.trim().to_lowercase().as_str(),
                "未知" | "待确认" | "待整理" | "待补齐" | "unknown" | "n/a" | "null" | "—"
            )
    }) {
        AspectCompletion::filled()
    } else {
        AspectCompletion::default()
    }
}
pub(super) fn resolve(
    reply: Option<&Description>,
    previous: Option<&FieldNote>,
    direct: Option<String>,
) -> Resolved {
    let prior = previous.map(|n| {
        n.context
            .completion
            .as_deref()
            .cloned()
            .unwrap_or_else(|| FieldCompletion {
                name: aspect(n.context.display_name.as_deref(), None),
                description: aspect(Some(&n.text), None),
            })
    });
    let keep_name = previous
        .zip(prior.as_ref())
        .filter(|(n, c)| c.name.addressed(n.context.display_name.as_deref()));
    let keep_description = previous
        .zip(prior.as_ref())
        .filter(|(n, c)| c.description.addressed(Some(&n.text)));
    let name = clean(direct.as_deref(), 40)
        .or_else(|| keep_name.and_then(|(n, _)| n.context.display_name.clone()))
        .or_else(|| clean(reply.and_then(|r| r.display_name.as_deref()), 40));
    let text = keep_description
        .map(|(n, _)| n.text.clone())
        .or_else(|| clean(reply.and_then(|r| r.description.as_deref()), 160))
        .unwrap_or_default();
    let completion = FieldCompletion {
        name: if direct.is_some() {
            AspectCompletion::filled()
        } else {
            keep_name.map(|(_, c)| c.name.clone()).unwrap_or_else(|| {
                aspect(
                    name.as_deref(),
                    reply.and_then(|r| r.name_unknown.as_deref()),
                )
            })
        },
        description: keep_description
            .map(|(_, c)| c.description.clone())
            .unwrap_or_else(|| {
                aspect(
                    Some(&text),
                    reply.and_then(|r| r.description_unknown.as_deref()),
                )
            }),
    };
    Resolved {
        name,
        text,
        completion,
        preserve_description: keep_description.is_some(),
    }
}
