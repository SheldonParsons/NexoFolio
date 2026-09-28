//! Whether a service address belongs to the project (0003 §4.2).

use nexofolio_contracts::endpoint::{AutoReason, ServiceAddress, Verdict};
use nexofolio_contracts::observation::CanonicalObservation;
use nexofolio_contracts::observation::{Fact, Headers};
use nexofolio_observe_contracts::AutoVerdict;

use crate::structure::clearly_not_json;

/// Another project count from which an address is a shared service: with
/// this one, three projects call it.
const SHARED_BY_OTHERS: u64 = 2;

/// The automatic verdict for the first call seen on `address`. A manual
/// verdict, when there is one, is applied by the caller.
pub fn auto_verdict(
    address: &ServiceAddress,
    observation: &CanonicalObservation,
    other_projects: u64,
) -> AutoVerdict {
    let decided = |verdict, reason| AutoVerdict { verdict, reason };
    if page_of(observation).is_some_and(|page| same_site(address.as_str(), &page)) {
        return decided(Verdict::Own, AutoReason::SameSite);
    }
    if other_projects >= SHARED_BY_OTHERS {
        return decided(Verdict::External, AutoReason::SharedAcrossProjects);
    }
    let Fact::Exchange(exchange) = &observation.fact else {
        return decided(Verdict::Own, AutoReason::Default);
    };
    if exchange
        .response
        .as_ref()
        .is_some_and(|response| clearly_not_json(&response.body))
    {
        return decided(Verdict::External, AutoReason::NotJson);
    }
    decided(Verdict::Own, AutoReason::Default)
}

/// The page the call was made from: the batch's site, else `Origin`, else
/// `Referer`.
fn page_of(observation: &CanonicalObservation) -> Option<String> {
    if let Some(site) = &observation.site {
        return Some(site.origin().to_owned());
    }
    let Fact::Exchange(exchange) = &observation.fact else {
        return None;
    };
    let headers = exchange.request.headers.as_ref()?;
    header(headers, "origin")
        .filter(|origin| *origin != "null")
        .or_else(|| header(headers, "referer"))
        .map(str::to_owned)
}

fn header<'a>(headers: &'a Headers, name: &str) -> Option<&'a str> {
    headers
        .entries
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let host = match authority.strip_prefix('[') {
        Some(ipv6) => ipv6.split(']').next()?,
        None => authority.split(':').next()?,
    };
    (!host.is_empty()).then(|| host.trim_end_matches('.').to_ascii_lowercase())
}

/// Same registrable domain (`api.shop.example.co.uk` and
/// `www.example.co.uk`). IPs and single-label hosts must match exactly.
fn same_site(a: &str, b: &str) -> bool {
    let (Some(a), Some(b)) = (host_of(a), host_of(b)) else {
        return false;
    };
    if a == b {
        return true;
    }
    let plain = |host: &str| host.contains('.') && host.parse::<std::net::IpAddr>().is_err();
    if !plain(&a) || !plain(&b) {
        return false;
    }
    match (psl::domain_str(&a), psl::domain_str(&b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexofolio_common::{EnvironmentId, ProjectId};
    use nexofolio_contracts::observation::{Body, Content};
    use nexofolio_contracts::scope::SiteScope;
    use nexofolio_contracts::testing::sample_exchange;

    fn call(url: &str, headers: &[(&str, &str)], response: Body) -> CanonicalObservation {
        let mut observation = sample_exchange(ProjectId::new(), EnvironmentId::new());
        let Fact::Exchange(exchange) = &mut observation.fact else {
            unreachable!()
        };
        exchange.request.url = url.into();
        exchange.request.headers = Some(Headers {
            complete: true,
            entries: headers
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        });
        exchange.response.as_mut().unwrap().body = response;
        observation
    }

    fn json() -> Body {
        Body::Full {
            media_type: Some("application/json".into()),
            content: Content::Text("{}".into()),
        }
    }

    fn html() -> Body {
        Body::Full {
            media_type: Some("text/html".into()),
            content: Content::Text("<html></html>".into()),
        }
    }

    fn verdict(url: &str, headers: &[(&str, &str)], response: Body, others: u64) -> AutoVerdict {
        let address = ServiceAddress::of_url(url).unwrap();
        auto_verdict(&address, &call(url, headers, response), others)
    }

    #[test]
    fn same_registrable_domain_is_own_even_for_files() {
        let own = verdict(
            "https://files.shop.example.co.uk/report.pdf",
            &[("Referer", "https://www.example.co.uk/page")],
            html(),
            5,
        );
        assert_eq!(
            own,
            AutoVerdict {
                verdict: Verdict::Own,
                reason: AutoReason::SameSite
            }
        );
        let mut observation = call("https://api.example.com/x", &[], html());
        observation.site = Some(SiteScope::new("https://app.example.com", "/").unwrap());
        let address = ServiceAddress::parse("https://api.example.com").unwrap();
        assert_eq!(
            auto_verdict(&address, &observation, 0).reason,
            AutoReason::SameSite
        );
    }

    #[test]
    fn origin_comes_before_referer() {
        let headers = [
            ("origin", "https://app.example.org"),
            ("referer", "https://app.example.com/"),
        ];
        let result = verdict("https://api.example.com/x", &headers, json(), 0);
        assert_eq!(result.reason, AutoReason::Default);
    }

    #[test]
    fn shared_or_non_json_addresses_are_external() {
        let page = [("Origin", "https://app.example.com")];
        let shared = verdict("https://analytics.example.net/c", &page, json(), 2);
        assert_eq!(shared.reason, AutoReason::SharedAcrossProjects);
        assert_eq!(
            verdict("https://analytics.example.net/c", &page, json(), 1).reason,
            AutoReason::Default
        );
        let pixel = verdict("https://cdn.example.net/p.gif", &page, html(), 0);
        assert_eq!(
            pixel,
            AutoVerdict {
                verdict: Verdict::External,
                reason: AutoReason::NotJson
            }
        );
        assert_eq!(
            verdict("https://cdn.example.net/p", &page, Body::None, 0).verdict,
            Verdict::Own
        );
    }

    #[test]
    fn ips_and_local_hosts_match_exactly() {
        assert!(same_site(
            "http://127.0.0.1:8080",
            "http://127.0.0.1:3000/page"
        ));
        assert!(!same_site("http://10.0.0.1", "http://10.0.0.2"));
        assert!(same_site("http://localhost:8080", "http://localhost/"));
        assert!(!same_site("http://intranet", "http://localhost"));
        assert!(!same_site("https://a.example.com", "not a url"));
    }
}
