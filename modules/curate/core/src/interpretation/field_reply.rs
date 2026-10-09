//! Validate model expression separately from direct evidence and unknown constraints.
use crate::FieldInput;
use nexofolio_contracts::knowledge::{Confirmation, EvidenceKind, FieldMeaning};

pub(super) fn direct_meaning(input: &FieldInput) -> (Option<String>, FieldMeaning) {
    let label = input
        .evidence
        .iter()
        .find(|e| e.kind == EvidenceKind::DirectSource)
        .and_then(|e| e.options.first())
        .and_then(|o| o.label.clone());
    let mut meaning = FieldMeaning::default();
    let display = label.as_ref().map(|label| {
        if label.ends_with("（万）") || label.ends_with("(万)") {
            meaning.unit = Some("万".into());
            meaning.unit_confirmation = Confirmation::Direct;
            meaning.unknowns.push("币种未确认".into());
            label
                .trim_end_matches("（万）")
                .trim_end_matches("(万)")
                .to_owned()
        } else {
            label.clone()
        }
    });
    if input.path == "code"
        && input.siblings.iter().any(|s| s == "msg")
        && input.siblings.iter().any(|s| s == "data")
        && matches!(
            input.location,
            nexofolio_contracts::endpoint::FieldLocation::ResponseBody { .. }
        )
    {
        meaning.role = Some("响应信封业务码；成功及错误码含义须另行确认".into());
    }
    (display, meaning)
}
pub(super) fn constrain_text(input: &FieldInput, text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    if input.path.to_ascii_lowercase().contains("sjb") && text.contains("设计部") {
        "技术回复相关内容；Sjb 部门简称待确认".into()
    } else if input.path == "code"
        && input.siblings.iter().any(|s| s == "msg")
        && input.siblings.iter().any(|s| s == "data")
        && matches!(
            input.location,
            nexofolio_contracts::endpoint::FieldLocation::ResponseBody { .. }
        )
    {
        "业务响应码；具体码值含义待确认".into()
    } else {
        text.trim().chars().take(160).collect()
    }
}
