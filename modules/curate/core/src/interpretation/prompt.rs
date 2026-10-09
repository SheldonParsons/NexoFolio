//! Prompts for each pass.

use nexofolio_contracts::endpoint::EndpointFacts;

pub fn for_description(facts: &EndpointFacts) -> String {
    let method = &facts.summary.method;
    let path = &facts.summary.path_template;

    let fields_summary = if facts.fields.is_empty() {
        "无字段信息".to_string()
    } else {
        facts
            .fields
            .iter()
            .take(64)
            .map(|f| {
                let location = match f.location {
                    nexofolio_contracts::endpoint::FieldLocation::RequestHeader => "请求 Header",
                    nexofolio_contracts::endpoint::FieldLocation::ResponseHeader { .. } => {
                        "响应 Header"
                    }
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
1. name: 动作 + 业务对象 + 必要范围（管理端/业务端须有路径和采样依据），合理旧名保留
2. purpose: 一句话用途说明\n3. description: 主要输入输出、ID 指向对象、样本使用场景及限制。区分观察/推测/配置，不凭 HTTP 200 宣称业务成功；不编造必调顺序或缩写含义。unknowns: 未确认事项数组

JSON 格式返回：
{{"name": "...", "purpose": "...", "description": "...", "unknowns": []}}

只返回 JSON，无其他内容。"#,
        method, path, fields_summary
    )
}
