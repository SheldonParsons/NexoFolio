//! Path templates: the path part of an endpoint's identity.
//!
//! Own addresses get relative templates (`/order/{id}`); external ones carry
//! the address (`https://analytics.example.net/collect`), so the same path on
//! two services never becomes one endpoint.

use nexofolio_contracts::endpoint::ServiceAddress;

/// Non-empty segments; the query is not part of a path.
pub fn segments(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn join(segments: &[&str]) -> String {
    format!("/{}", segments.join("/"))
}

pub fn is_param(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    let all = |test: fn(&u8) -> bool| bytes.iter().all(test);
    let braced = segment.len() > 2 && segment.starts_with('{') && segment.ends_with('}');
    let digits = all(u8::is_ascii_digit);
    let hex = bytes.len() >= 16 && all(u8::is_ascii_hexdigit);
    let uuid = bytes.len() == 36
        && bytes.iter().enumerate().all(|(at, byte)| match at {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        });
    let date = bytes.len() == 10
        && bytes.iter().enumerate().all(|(at, byte)| match at {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        });
    // One short digit run is a word with a version, like `pageListV2`, not a token.
    let digit_runs: Vec<&[u8]> = bytes
        .split(|byte| !byte.is_ascii_digit())
        .filter(|run| !run.is_empty())
        .collect();
    let word = digit_runs.len() <= 1 && digit_runs.iter().all(|run| run.len() <= 3);
    let token = bytes.len() >= 20
        && all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_')
        && bytes.iter().any(u8::is_ascii_alphabetic)
        && !word;
    braced || digits || hex || uuid || date || token
}

/// Parameter-looking segments become `{id}`, `{id2}`, … in order.
pub fn generic(path: &str) -> String {
    let mut params = 0;
    let named: Vec<String> = segments(path)
        .into_iter()
        .map(|segment| {
            if !is_param(segment) {
                return segment.to_owned();
            }
            params += 1;
            match params {
                1 => "{id}".to_owned(),
                n => format!("{{id{n}}}"),
            }
        })
        .collect();
    format!("/{}", named.join("/"))
}

/// A declared template as observe keys it; `None` if it is not a path.
pub fn declared(template: &str) -> Option<String> {
    template.starts_with('/').then(|| join(&segments(template)))
}

/// Parameter names of a template, in order.
pub fn params(template: &str) -> Vec<&str> {
    segments(template)
        .into_iter()
        .filter_map(|segment| segment.strip_prefix('{')?.strip_suffix('}'))
        .collect()
}

pub fn external(address: &ServiceAddress, path: &str) -> String {
    format!("{address}{}", generic(path))
}

#[derive(Debug, PartialEq, Eq)]
pub struct Own {
    pub template: String,
    /// A base path learnt from this path, when none was known.
    pub new_base: Option<String>,
}

/// The template of a path on an own address. The known base path is cut off
/// first; then the most specific declared template matching the path's tail
/// wins. Without a known base, an all-literal leading remainder becomes it.
pub fn own(path: &str, base: Option<&str>, declared: &[&str]) -> Own {
    let full = segments(path);
    let (path, may_learn_base) = match base.map(segments) {
        Some(base) if full.starts_with(&base) => (&full[base.len()..], false),
        Some(_) => (&full[..], false),
        None => (&full[..], true),
    };
    let mut best: Option<((usize, usize), &str, usize)> = None;
    for template in declared {
        let wanted = segments(template);
        let Some(prefix) = path.len().checked_sub(wanted.len()) else {
            continue;
        };
        let literals = wanted.iter().filter(|segment| !is_braced(segment)).count();
        let prefix_ok = prefix == 0
            || (may_learn_base && literals > 0 && !path[..prefix].iter().any(|s| is_param(s)));
        let tail_ok = path[prefix..]
            .iter()
            .zip(&wanted)
            .all(|(actual, wanted)| is_braced(wanted) || actual == wanted);
        let rank = (wanted.len(), literals);
        if prefix_ok && tail_ok && best.is_none_or(|(best, _, _)| rank > best) {
            best = Some((rank, template, prefix));
        }
    }
    match best {
        Some((_, template, prefix)) => Own {
            template: (*template).to_owned(),
            new_base: (prefix > 0).then(|| join(&path[..prefix])),
        },
        None => Own {
            template: generic(&join(path)),
            new_base: None,
        },
    }
}

fn is_braced(segment: &str) -> bool {
    segment.starts_with('{') && segment.ends_with('}')
}

/// The path an endpoint's traffic on `address` was made of, as far as the
/// template still tells: external templates lose their address, own ones get
/// the base path back unless they already start with it.
pub fn path_of(template: &str, address: &ServiceAddress, base: Option<&str>) -> String {
    if let Some(path) = template.strip_prefix(address.as_str()) {
        return path.to_owned();
    }
    match base {
        Some(base) if !segments(template).starts_with(&segments(base)) => {
            join(&[segments(base), segments(template)].concat())
        }
        _ => template.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_parameter_segments() {
        for param in [
            "1001",
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
            "0123456789abcdef",
            "2026-09-28",
            "a1B2c3D4e5F6g7H8i9J0",
            "tok_12345678901234567890",
            "{orderId}",
        ] {
            assert!(is_param(param), "{param}");
        }
        for literal in [
            "order",
            "v2",
            "items",
            "abcdef",
            "a-very-long-literal-name",
            "commodityPageForConsultOrderV2",
            "getCategoryTreeWithinUserProductLinePermissionsV2",
            "getOrderV2ListByCustomerId",
            "{}",
        ] {
            assert!(!is_param(literal), "{literal}");
        }
    }

    #[test]
    fn generic_templates_name_params_in_order() {
        assert_eq!(
            generic("/api/order/1001/items/7"),
            "/api/order/{id}/items/{id2}"
        );
        assert_eq!(generic("//api//order/"), "/api/order");
        assert_eq!(generic("/"), "/");
        assert_eq!(
            generic("/user/alice"),
            "/user/alice",
            "values alone stay (2f)"
        );
        assert_eq!(generic("/order/{orderId}"), "/order/{id}");
    }

    #[test]
    fn external_templates_carry_the_address() {
        let address = ServiceAddress::parse("https://analytics.example.net").unwrap();
        assert_eq!(
            external(&address, "/collect/123"),
            "https://analytics.example.net/collect/{id}"
        );
        assert_eq!(
            path_of(
                "https://analytics.example.net/collect/{id}",
                &address,
                Some("/api")
            ),
            "/collect/{id}"
        );
    }

    #[test]
    fn declared_tail_wins_and_teaches_the_base_path() {
        let declared = ["/customer/{customerId}", "/order/{orderId}"];
        let result = own("/api/customer/1001", None, &declared);
        assert_eq!(
            result,
            Own {
                template: "/customer/{customerId}".into(),
                new_base: Some("/api".into())
            }
        );
        let later = own("/api/cart/7", Some("/api"), &declared);
        assert_eq!(
            later.template, "/cart/{id}",
            "the base is cut before generic rules"
        );
        assert_eq!(later.new_base, None);
        let elsewhere = own("/v2/customer/1", Some("/api"), &declared);
        assert_eq!(elsewhere.template, "/v2/customer/{id}", "no second base");
    }

    #[test]
    fn the_most_specific_declaration_wins() {
        let declared = ["/order/{orderId}", "/order/latest", "/{kind}/latest"];
        assert_eq!(
            own("/order/latest", None, &declared).template,
            "/order/latest"
        );
        assert_eq!(
            own("/order/5", None, &declared).template,
            "/order/{orderId}"
        );
    }

    #[test]
    fn prefixes_must_be_literal_and_templates_must_have_literals() {
        let declared = ["/items", "/{id}"];
        assert_eq!(
            own("/order/5/items", None, &declared).template,
            "/order/{id}/items"
        );
        assert_eq!(own("/api/5", None, &declared).template, "/api/{id}");
        assert_eq!(own("/5", None, &declared).template, "/{id}");
    }

    #[test]
    fn path_of_restores_the_base_once() {
        let address = ServiceAddress::parse("https://api.example.com").unwrap();
        assert_eq!(
            path_of("/customer/{id}", &address, Some("/api")),
            "/api/customer/{id}"
        );
        assert_eq!(
            path_of("/api/customer/{id}", &address, Some("/api")),
            "/api/customer/{id}"
        );
        assert_eq!(path_of("/", &address, Some("/api")), "/api");
        assert_eq!(path_of("/customer/{id}", &address, None), "/customer/{id}");
    }
}
