//! Generic protocol/structure meaning only. No business constraints or credential claims.
use crate::FieldInput;
use nexofolio_contracts::{
    endpoint::{FieldLocation, ValueType},
    knowledge::EvidenceKind,
};
pub(crate) struct StandardMeaning {
    pub name: &'static str,
    pub text: &'static str,
    pub reference: &'static str,
}
pub(crate) fn standard_meaning(input: &FieldInput) -> Option<StandardMeaning> {
    let header = matches!(
        input.location,
        FieldLocation::RequestHeader | FieldLocation::ResponseHeader { .. }
    );
    if input.evidence.iter().any(|e| {
        e.kind == EvidenceKind::DirectSource
            || (e.kind == EvidenceKind::RelatedInterface
                && !(header
                    && e.options.is_empty()
                    && e.source.as_ref().and_then(|s| s.location) == Some(input.location)
                    && e.text
                        .ends_with("观测到同名同值；候选关联，未确认值来源或调用依赖")))
    }) {
        return None;
    }
    if !header {
        if !input.path.is_empty()
            || input.types.is_empty()
            || input
                .types
                .iter()
                .any(|t| !matches!(t, ValueType::Object | ValueType::Array))
        {
            return None;
        }
        return match input.location {
            FieldLocation::RequestBody => Some(StandardMeaning {
                name: "请求报文",
                text: "请求体的顶层对象或数组；子字段按采集到的实际结构展开。",
                reference: "采集到的请求体根节点结构",
            }),
            FieldLocation::ResponseBody { .. } => Some(StandardMeaning {
                name: "响应报文",
                text: "响应体的顶层对象或数组；子字段按采集到的实际结构展开，不据此判断业务是否成功。",
                reference: "采集到的响应体根节点结构",
            }),
            _ => None,
        };
    }
    if input.types != [ValueType::String] {
        return None;
    }
    let request = input.location == FieldLocation::RequestHeader;
    let http = "https://www.rfc-editor.org/rfc/rfc9110.html";
    let cache = "https://www.rfc-editor.org/rfc/rfc9111.html";
    let (name, text, reference) = match input.path.to_ascii_lowercase().as_str() {
        "accept" if request => (
            "可接受的响应媒体类型",
            "声明客户端可接受的响应媒体类型及偏好；实际返回格式以响应为准。",
            http,
        ),
        "authorization" if request => (
            "认证信息",
            "携带 HTTP 认证方案及认证信息；具体认证要求和凭证获取方式需结合该接口确认，历史值不代表当前有效。",
            http,
        ),
        "content-type" => (
            "内容媒体类型",
            "标识消息内容的媒体类型及相关参数，用于说明正文应如何解析。",
            http,
        ),
        "content-length" => (
            "内容字节数",
            "声明消息内容的字节数，具体适用方式取决于消息语义与传输方式。",
            http,
        ),
        "content-encoding" => (
            "内容编码",
            "标识应用于表示数据的内容编码，例如压缩方式；与媒体类型含义不同。",
            http,
        ),
        "cache-control" => (
            "缓存控制指令",
            "传达请求或响应的缓存处理指令；实际策略需要结合具体指令值及其他缓存条件判断。",
            cache,
        ),
        "date" if !request => (
            "消息生成时间",
            "表示消息生成时刻的 HTTP 日期；不等同于业务数据的创建或更新时间。",
            http,
        ),
        "expires" if !request => (
            "缓存过期时间",
            "给出响应的新鲜度过期时间；需要同时考虑 Cache-Control 等缓存规则。",
            cache,
        ),
        "pragma" => (
            "兼容性缓存指令",
            "用于兼容旧版 HTTP 的指令字段；响应中的 Pragma 语义不能据此等同于 Cache-Control。",
            cache,
        ),
        "vary" if !request => (
            "缓存区分依据",
            "指出选择响应时参考的请求头字段，缓存据此区分可复用的响应。",
            http,
        ),
        "etag" if !request => (
            "表示版本标识",
            "表示所选资源表示的实体标签，可用于条件请求和缓存验证；不是业务对象编号。",
            http,
        ),
        "last-modified" if !request => (
            "表示最后修改时间",
            "给出所选资源表示的最后修改时刻，可用于条件请求；不必等同于业务记录更新时间。",
            http,
        ),
        "x-frame-options" if !request => (
            "框架嵌入策略",
            "声明浏览器对页面框架嵌入的限制意图；实际效果取决于指令、浏览器及其他策略。",
            "https://www.rfc-editor.org/rfc/rfc7034.html",
        ),
        "x-content-type-options" if !request => (
            "媒体类型检查策略",
            "用于要求浏览器按声明的媒体类型检查响应；具体处理取决于该头的取值与请求用途。",
            "https://fetch.spec.whatwg.org/#x-content-type-options-header",
        ),
        "referrer-policy" if !request => (
            "来源信息发送策略",
            "规定浏览器从该资源上下文发起请求时，如何携带 Referer 来源信息。",
            "https://www.w3.org/TR/referrer-policy/#referrer-policy-header",
        ),
        _ => return None,
    };
    Some(StandardMeaning {
        name,
        text,
        reference,
    })
}
