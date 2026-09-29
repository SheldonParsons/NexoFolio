use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use nexofolio_common::{EndpointId, ProjectId};
use serde_json::Value;

use crate::endpoint::{
    AddressStatus, Decision, EndpointError, EndpointFacts, EndpointReader, EndpointSummary,
    ServiceAddress, ServiceAddresses, Verdict,
};

/// Fake observe read model, seeded with ready-made facts.
#[derive(Default)]
pub struct InMemoryEndpoints {
    facts: Mutex<Vec<EndpointFacts>>,
    samples: Mutex<HashMap<(EndpointId, String), Value>>,
}

impl InMemoryEndpoints {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces the endpoint with the same ID.
    pub fn put(&self, facts: EndpointFacts) {
        let mut all = self.facts.lock().expect("fake lock");
        all.retain(|existing| existing.summary.id != facts.summary.id);
        all.push(facts);
    }

    /// Sets the raw sample behind one of the endpoint's example IDs.
    pub fn put_example(&self, endpoint: EndpointId, example: &str, sample: Value) {
        self.samples
            .lock()
            .expect("fake lock")
            .insert((endpoint, example.to_owned()), sample);
    }
}

#[async_trait]
impl EndpointReader for InMemoryEndpoints {
    async fn list(&self, project: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        let all = self.facts.lock().expect("fake lock");
        Ok(all
            .iter()
            .filter(|facts| facts.summary.project_id == project)
            .map(|facts| facts.summary.clone())
            .collect())
    }

    async fn get(&self, id: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        let all = self.facts.lock().expect("fake lock");
        Ok(all
            .iter()
            .find(|facts| facts.summary.id == id || facts.aliases.contains(&id))
            .cloned())
    }

    async fn example(&self, id: EndpointId, example: &str) -> Result<Option<Value>, EndpointError> {
        let Some(facts) = self.get(id).await? else {
            return Ok(None);
        };
        let samples = self.samples.lock().expect("fake lock");
        Ok(samples
            .get(&(facts.summary.id, example.to_owned()))
            .cloned())
    }
}

/// Fake observe address book: manual verdicts only, no traffic.
#[derive(Default)]
pub struct InMemoryAddresses {
    manual: Mutex<HashMap<(ProjectId, ServiceAddress), Verdict>>,
}

impl InMemoryAddresses {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ServiceAddresses for InMemoryAddresses {
    async fn list(&self, project: ProjectId) -> Result<Vec<AddressStatus>, EndpointError> {
        let manual = self.manual.lock().expect("fake lock");
        let mut listed: Vec<_> = manual
            .iter()
            .filter(|((owner, _), _)| *owner == project)
            .map(|((_, address), verdict)| AddressStatus {
                address: address.clone(),
                verdict: *verdict,
                decision: Decision::Manual,
                calls: 0,
                last_seen: None,
            })
            .collect();
        listed.sort_by(|a, b| a.address.cmp(&b.address));
        Ok(listed)
    }

    async fn decide(
        &self,
        project: ProjectId,
        address: ServiceAddress,
        verdict: Option<Verdict>,
    ) -> Result<(), EndpointError> {
        let mut manual = self.manual.lock().expect("fake lock");
        match verdict {
            Some(verdict) => manual.insert((project, address), verdict),
            None => manual.remove(&(project, address)),
        };
        Ok(())
    }
}

/// Behaviour every [`ServiceAddresses`] must show for addresses without
/// traffic. Uses fresh project IDs. Panics on the first violation.
pub async fn service_addresses_conformance<S: ServiceAddresses>(addresses: &S) {
    let project = ProjectId::new();
    let other = ProjectId::new();
    let analytics = ServiceAddress::parse("https://analytics.example.net").expect("address");
    let api = ServiceAddress::parse("https://api.example.com").expect("address");
    let list = |project| async move { addresses.list(project).await.expect("list") };

    assert!(list(project).await.is_empty(), "new project");

    addresses
        .decide(project, analytics.clone(), Some(Verdict::External))
        .await
        .expect("register before traffic");
    addresses
        .decide(project, api.clone(), Some(Verdict::Own))
        .await
        .expect("register second");
    let registered = list(project).await;
    assert_eq!(registered.len(), 2);
    let status = registered
        .iter()
        .find(|status| status.address == analytics)
        .expect("listed");
    assert_eq!(status.verdict, Verdict::External);
    assert_eq!(status.decision, Decision::Manual);
    assert_eq!(
        (status.calls, status.last_seen),
        (0, None),
        "no traffic yet"
    );

    addresses
        .decide(project, analytics.clone(), Some(Verdict::Own))
        .await
        .expect("change verdict");
    let changed = list(project).await;
    assert_eq!(changed.len(), 2, "one entry per address");
    assert!(
        changed
            .iter()
            .any(|status| status.address == analytics && status.verdict == Verdict::Own)
    );

    assert!(list(other).await.is_empty(), "verdicts are per project");

    addresses
        .decide(project, analytics.clone(), None)
        .await
        .expect("hand back");
    let remaining = list(project).await;
    assert_eq!(remaining.len(), 1, "unseen address without verdict is gone");
    assert_eq!(remaining[0].address, api);
    addresses
        .decide(project, analytics, None)
        .await
        .expect("handing back twice is harmless");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_passes_service_addresses_conformance() {
        service_addresses_conformance(&InMemoryAddresses::new()).await;
    }

    #[tokio::test]
    async fn fake_reader_resolves_aliases() {
        let reader = InMemoryEndpoints::new();
        let project = ProjectId::new();
        let (id, old) = (EndpointId::new(), EndpointId::new());
        reader.put(EndpointFacts {
            summary: EndpointSummary {
                id,
                project_id: project,
                method: "GET".into(),
                path_template: "/order/{id}".into(),
                external: false,
                declared: false,
                environments: Vec::new(),
            },
            aliases: vec![old],
            addresses: Vec::new(),
            fields: Vec::new(),
            examples: Vec::new(),
        });
        reader.put_example(id, "e1", serde_json::json!({ "sample": 1 }));
        assert_eq!(reader.list(project).await.unwrap().len(), 1);
        assert!(reader.list(ProjectId::new()).await.unwrap().is_empty());
        assert_eq!(
            reader.get(old).await.unwrap().map(|facts| facts.summary.id),
            Some(id)
        );
        assert_eq!(reader.get(EndpointId::new()).await.unwrap(), None);
        assert_eq!(
            reader.example(old, "e1").await.unwrap(),
            Some(serde_json::json!({ "sample": 1 })),
            "examples resolve aliases too"
        );
        assert_eq!(reader.example(id, "e2").await.unwrap(), None);
    }
}
