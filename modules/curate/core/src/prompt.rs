//! Prompts for each pass.

use nexofolio_contracts::endpoint::{EndpointFacts, EndpointSummary};

pub fn for_description(facts: &EndpointFacts) -> String {
    let method = &facts.summary.method;
    let path = &facts.summary.path_template;

    let fields_summary = if facts.fields.is_empty() {
        "无字段信息".to_string()
    } else {
        facts
            .fields
            .iter()
            .take(10)
            .map(|f| {
                let location = match f.location {
                    nexofolio_contracts::endpoint::FieldLocation::RequestBody => "请求体",
                    nexofolio_contracts::endpoint::FieldLocation::ResponseBody { .. } => "响应体",
                    nexofolio_contracts::endpoint::FieldLocation::Query => "查询参数",
                    nexofolio_contracts::endpoint::FieldLocation::Path => "路径参数",
                };
                let types_str = f
                    .types
                    .iter()
                    .map(|t| format!("{:?}", t))
                    .collect::<Vec<_>>()
                    .join("|");
                format!("{}: {} ({})", f.path, types_str, location)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"你是 API 文档专家。根据接口的方法、路径和字段，生成简洁的中文描述。

方法: {}
路径: {}
字段:
{}

生成：
1. name: 简短的中文名称（不超过 15 字），说明这个接口做什么
2. purpose: 一句话用途说明（不超过 30 字）

JSON 格式返回：
{{"name": "...", "purpose": "..."}}

只返回 JSON，无其他内容。"#,
        method, path, fields_summary
    )
}

pub fn for_organization(summaries: &[EndpointSummary]) -> String {
    let paths: Vec<String> = summaries
        .iter()
        .map(|s| format!("{} {}", s.method, s.path_template))
        .collect();

    let sample = if paths.len() > 30 {
        paths.iter().take(30).cloned().collect::<Vec<_>>().join("\n")
            + &format!("\n... 还有 {} 个接口", paths.len() - 30)
    } else {
        paths.join("\n")
    };

    format!(
        r#"你是 API 架构专家。根据项目的接口列表，设计一个合理的文件夹分类结构。

接口列表:
{}

要求：
1. 设计 3-8 个顶级文件夹，按业务模块划分
2. 每个文件夹包含：name（中文名称）、summary（一句话说明）、match（路径中的关键片段列表）
3. match 里的每个字符串只要出现在接口路径中，该接口就归入这个文件夹
4. 关键片段要具体，例如 "/orgUserTree/"、"/oauth/"，不要用 "/" 或 "api" 这类会匹配一切的片段

JSON 数组格式返回：
[
  {{"name": "用户与组织", "summary": "用户、组织树与权限查询", "match": ["/users/", "/orgUserTree/"]}},
  ...
]

只返回 JSON 数组，无其他内容。"#,
        sample
    )
}
