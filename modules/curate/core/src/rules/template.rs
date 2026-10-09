//! Bind a template only through the matched task's formKey; numeric codes alone are insufficient.
use super::navigation::{LinkProposal, NAVIGATION_RECIPE, scoped_before};
use crate::SignalIndex;
use nexofolio_contracts::{
    endpoint::{EndpointFacts, FieldLocation},
    knowledge::{Confirmation, FieldMapping, LinkContext, Relation},
};
pub(super) fn proposals(facts: &[EndpointFacts], index: &SignalIndex<'_>) -> Vec<LinkProposal> {
    let mut out = Vec::new();
    for target in facts {
        let own: Vec<_> = index
            .endpoint(target.summary.id)
            .filter(|s| {
                s.endpoint == target.summary.id
                    && !matches!(s.location, FieldLocation::ResponseBody { .. })
            })
            .collect();
        for form in index.name("formKey").filter(|s| {
            s.name == "formKey" && matches!(s.location, FieldLocation::ResponseBody { .. })
        }) {
            let task_matches = ["taskId", "processInstanceId"].iter().all(|name| {
                index.record(form).any(|s| {
                    s.endpoint == form.endpoint
                        && s.example == form.example
                        && s.row == form.row
                        && s.name == *name
                        && own
                            .iter()
                            .any(|t| t.name == *name && t.value == s.value && scoped_before(s, t))
                })
            });
            if !task_matches {
                continue;
            }
            for template in facts.iter().filter(|f| {
                f.summary
                    .path_template
                    .ends_with(&format!("/listByFormKey/{}", form.value))
            }) {
                for kind in index.endpoint(template.summary.id).filter(|s| {
                    s.endpoint == template.summary.id
                        && s.name == "type"
                        && matches!(s.location, FieldLocation::ResponseBody { .. })
                }) {
                    let name = index.record(kind).find(|s| {
                        s.endpoint == kind.endpoint
                            && s.example == kind.example
                            && s.row == kind.row
                            && s.name == "name"
                    });
                    let Some(name) = name else {
                        continue;
                    };
                    for (source, target_name) in [(kind, "actionType"), (name, "actionName")] {
                        let Some(input) = own.iter().find(|s| {
                            s.name == target_name
                                && s.value == source.value
                                && scoped_before(source, s)
                        }) else {
                            continue;
                        };
                        let comment = index
                            .record(kind)
                            .find(|s| {
                                s.endpoint == kind.endpoint
                                    && s.example == kind.example
                                    && s.row == kind.row
                                    && s.name == "approvalCommentIsRequired"
                            })
                            .map(|s| s.value.as_str())
                            .unwrap_or("unknown");
                        let upload = index
                            .record(kind)
                            .find(|s| {
                                s.endpoint == kind.endpoint
                                    && s.example == kind.example
                                    && s.row == kind.row
                                    && s.name == "uploadFileIsRequired"
                            })
                            .map(|s| s.value.as_str())
                            .unwrap_or("unknown");
                        let context = LinkContext {
                            source_input: None,
                            from_location: Some(source.location),
                            to_location: Some(input.location),
                            environment: Some(input.environment),
                            scope: Some(format!("formKey={}", form.value)),
                            condition: Some(
                                "仅适用对应 taskId/processInstanceId 指向的 formKey 及所选操作"
                                    .into(),
                            ),
                            confirmation: Confirmation::Direct,
                            sources: vec![
                                form.source.clone(),
                                source.source.clone(),
                                input.source.clone(),
                            ],
                            matched_fields: vec![
                                FieldMapping {
                                    from: kind.path.clone(),
                                    to: "actionType".into(),
                                },
                                FieldMapping {
                                    from: name.path.clone(),
                                    to: "actionName".into(),
                                },
                            ],
                            recipe: Some(NAVIGATION_RECIPE.into()),
                        };
                        out.push(LinkProposal {
                            from: source.endpoint,
                            from_field: source.path.clone(),
                            to: input.endpoint,
                            to_field: input.path.clone(),
                            relation: Relation::Dictionary,
                            evidence: format!(
                                "任务身份匹配的 formKey={} 模板选项；此选项意见要求={comment}，附件要求={upload}。配置不是已验证后台必填",
                                form.value),
                            context,
                        });
                    }
                }
            }
        }
    }
    out
}
