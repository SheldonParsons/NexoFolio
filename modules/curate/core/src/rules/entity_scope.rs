//! Conservative resource scope for a single-ID list/detail correspondence.
//! Sharing a gateway, service prefix or API version is not an entity match.
fn resource(path: &str) -> Option<Vec<&str>> {
    let ends_in_parameter = path.trim_end_matches('/').ends_with('}');
    let mut parts: Vec<_> = path
        .split('/')
        .filter(|p| !p.is_empty() && !p.contains('{'))
        .collect();
    let last = *parts.last()?;
    // API methods normally end in an operation. REST /resource/{id} instead
    // names the resource directly; only recognized operations are removed there.
    if !ends_in_parameter || ["detail", "find", "get", "view", "delete", "update"].contains(&last) {
        parts.pop();
    }
    let resource = *parts.last()?;
    let infrastructure = ["gateway", "api", "admin", "client", "manager", "oauth"];
    if infrastructure.contains(&resource)
        || resource.ends_with("-api")
        || resource
            .strip_prefix('v')
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    Some(parts)
}

pub(super) fn same_resource(source: &str, target: &str) -> bool {
    resource(source)
        .zip(resource(target))
        .is_some_and(|(a, b)| a == b)
}
