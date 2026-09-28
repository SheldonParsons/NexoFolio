//! `admin observe report`: what observe knows about one project, as plain
//! text. The stage 2 acceptance tool (0003 §3); stage 3's views replace it.

use chrono::{DateTime, Local, Utc};
use nexofolio_access_contracts::Environment;
use nexofolio_common::{EnvironmentId, Error, ProjectId, Result};
use nexofolio_contracts::endpoint::{
    AddressStatus, Conflict, Decision, EndpointReader, FieldFacts, FieldLabel, FieldLocation,
    FieldPath, PathSegment, ServiceAddresses, Verdict,
};
use std::{collections::BTreeMap, fmt::Write};

/// Fields nested deeper than this many keys are counted, not listed.
const SHOWN_DEPTH: usize = 3;

pub async fn observe_report<O>(
    observe: &O,
    project: ProjectId,
    environments: &[Environment],
) -> Result<String>
where
    O: EndpointReader + ServiceAddresses,
{
    let unavailable = |_| Error::Unavailable {
        component: "observe",
    };
    let addresses = ServiceAddresses::list(observe, project)
        .await
        .map_err(unavailable)?;
    let endpoints = EndpointReader::list(observe, project)
        .await
        .map_err(unavailable)?;
    let names = Names(environments);
    let mut out = String::new();
    let mut pending: Vec<String> = Vec::new();

    writeln!(out, "项目 {project}").unwrap();
    writeln!(out, "时间为本机时区（UTC{}）", Local::now().format("%:z")).unwrap();
    writeln!(out, "\n服务地址（{}）", addresses.len()).unwrap();
    for address in &addresses {
        writeln!(out, "  {}", describe_address(address)).unwrap();
    }

    writeln!(out, "\n接口（{}）", endpoints.len()).unwrap();
    for summary in &endpoints {
        let Some(facts) = observe.get(summary.id).await.map_err(unavailable)? else {
            continue;
        };
        let mut marks = Vec::new();
        if summary.declared {
            marks.push("已声明".to_owned());
        }
        if summary.external {
            marks.push("外部服务".to_owned());
        }
        if !facts.aliases.is_empty() {
            marks.push(format!("别名 {}", facts.aliases.len()));
        }
        writeln!(
            out,
            "\n{} {}{}",
            summary.method,
            summary.path_template,
            if marks.is_empty() {
                String::new()
            } else {
                format!("  [{}]", marks.join("，"))
            }
        )
        .unwrap();
        for usage in &summary.environments {
            writeln!(
                out,
                "  {}：{} 次，{} 至 {}",
                names.get(usage.environment_id),
                usage.calls,
                local(usage.first_seen),
                local(usage.last_seen)
            )
            .unwrap();
        }
        for used in &facts.addresses {
            if !used.base_path.is_empty() {
                writeln!(
                    out,
                    "  前缀 {}{}（{}）",
                    used.address,
                    used.base_path,
                    names.get(used.environment_id)
                )
                .unwrap();
            }
        }
        // Deep fields are folded into their ancestor at SHOWN_DEPTH, one line each.
        let mut folded: BTreeMap<(String, String), usize> = BTreeMap::new();
        for field in &facts.fields {
            match shown(&field.path) {
                None => writeln!(out, "    {}", describe_field(field, &names)).unwrap(),
                Some(ancestor) => {
                    *folded
                        .entry((location(field.location), ancestor.to_string()))
                        .or_default() += 1
                }
            }
            if let Some(conflict) = &field.conflict {
                pending.push(format!(
                    "{} {} 的 {} {}：{}",
                    summary.method,
                    summary.path_template,
                    location(field.location),
                    field.path,
                    describe_conflict(conflict)
                ));
            }
        }
        for ((location, ancestor), count) in folded {
            writeln!(out, "    {location} {ancestor} 下还有 {count} 个更深的字段").unwrap();
        }
    }

    writeln!(out, "\n待裁决（{}）", pending.len()).unwrap();
    for item in pending {
        writeln!(out, "  {item}").unwrap();
    }
    Ok(out)
}

/// Environment names by id; ids access doesn't know are printed as they are.
struct Names<'a>(&'a [Environment]);

impl Names<'_> {
    fn get(&self, id: EnvironmentId) -> String {
        self.0
            .iter()
            .find(|environment| environment.id == id)
            .map_or_else(
                || format!("环境 {id}"),
                |environment| environment.name.clone(),
            )
    }
}

/// `None` when the path is shallow enough to list; otherwise its ancestor at `SHOWN_DEPTH` keys.
fn shown(path: &FieldPath) -> Option<FieldPath> {
    let mut keys = 0;
    for (index, segment) in path.0.iter().enumerate() {
        if let PathSegment::Key(_) = segment {
            keys += 1;
            if keys > SHOWN_DEPTH {
                return Some(FieldPath(path.0[..index].to_vec()));
            }
        }
    }
    None
}

fn local(time: DateTime<Utc>) -> String {
    time.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

fn describe_address(address: &AddressStatus) -> String {
    let verdict = match address.verdict {
        Verdict::Own => "本项目",
        Verdict::External => "外部",
    };
    let decision = match address.decision {
        Decision::Manual => "人工",
        Decision::Default => "默认",
    };
    format!(
        "{} {verdict}（{decision}），{} 次",
        address.address, address.calls
    )
}

fn describe_field(field: &FieldFacts, names: &Names) -> String {
    let types: Vec<String> = field.types.iter().map(|t| snake(*t)).collect();
    // One label when every environment agrees; per environment only when they differ.
    let same = field
        .labels
        .windows(2)
        .all(|pair| pair[0].label == pair[1].label);
    let labels: Vec<String> = if same {
        field
            .labels
            .first()
            .map(|label| describe_label(label.label))
            .into_iter()
            .collect()
    } else {
        field
            .labels
            .iter()
            .map(|label| {
                format!(
                    "{} {}",
                    names.get(label.environment_id),
                    describe_label(label.label)
                )
            })
            .collect()
    };
    let mut line = format!(
        "{} {} {}",
        location(field.location),
        if field.path.0.is_empty() {
            "(整体)".to_owned()
        } else {
            field.path.to_string()
        },
        types.join("|")
    );
    if !labels.is_empty() {
        write!(line, "  {}", labels.join("；")).unwrap();
    }
    if field.differs_between_environments {
        line.push_str("  [环境不同]");
    }
    if let Some(declared) = &field.declared {
        line.push_str(if declared.required {
            "  [声明：必填]"
        } else {
            "  [声明：可选]"
        });
    }
    line
}

fn describe_label(label: FieldLabel) -> String {
    match label {
        FieldLabel::Observing => "观察中".into(),
        FieldLabel::Always => "总是出现".into(),
        FieldLabel::Optional => "可选".into(),
        FieldLabel::Added { since } => format!("自 {} 起新增", local(since)),
        FieldLabel::Removed { since } => format!("自 {} 起消失", local(since)),
        FieldLabel::Polymorphic => "多种类型".into(),
        FieldLabel::Absent => "从未出现".into(),
    }
}

fn describe_conflict(conflict: &Conflict) -> String {
    match conflict {
        Conflict::RequiredButAbsent { absent_calls } => {
            format!("声明必填，但有 {absent_calls} 次调用没有")
        }
        Conflict::TypeMismatch { observed } => {
            let observed: Vec<String> = observed.iter().map(|t| snake(*t)).collect();
            format!("类型与声明不符，实际为 {}", observed.join("|"))
        }
    }
}

fn location(location: FieldLocation) -> String {
    match location {
        FieldLocation::Path => "路径".into(),
        FieldLocation::Query => "查询".into(),
        FieldLocation::RequestBody => "请求体".into(),
        FieldLocation::ResponseBody { status } => format!("响应 {status}"),
    }
}

/// The serde name of a unit enum value, e.g. `string`.
fn snake<T: serde::Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}
