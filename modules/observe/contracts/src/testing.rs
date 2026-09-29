//! In-memory store and the behaviour every [`ObserveStore`] must show.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::endpoint::{
    EndpointChange, EndpointEvent, EnvironmentUsage, ServiceAddress, Verdict,
};
use nexofolio_contracts::feed::{Change, ChangeFeed, Cursor, FeedError};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::*;

#[derive(Clone, Default)]
struct State {
    seen: HashMap<Uuid, DateTime<Utc>>,
    addresses: HashMap<(ProjectId, ServiceAddress), AddressRow>,
    base_paths: HashMap<(ProjectId, EnvironmentId, ServiceAddress), String>,
    /// Endpoint and the endpoint it was merged into.
    endpoints: Vec<(EndpointRow, Option<EndpointId>)>,
    fingerprints: Vec<StoredFingerprint>,
    declarations: Vec<(EndpointId, Declaration)>,
    outbox: Vec<Change<EndpointEvent>>,
}

#[derive(Clone)]
struct StoredFingerprint {
    traffic: Traffic,
    hash: FingerprintHash,
    structure: Value,
    sample: Value,
    calls: u64,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
}

impl State {
    fn current(&self, project: ProjectId) -> impl Iterator<Item = &EndpointRow> {
        self.endpoints
            .iter()
            .filter(move |(row, into)| row.project_id == project && into.is_none())
            .map(|(row, _)| row)
    }

    fn project_of(&self, endpoint: EndpointId) -> Option<ProjectId> {
        self.endpoints
            .iter()
            .find(|(row, into)| row.id == endpoint && into.is_none())
            .map(|(row, _)| row.project_id)
    }
}

fn now() -> DateTime<Utc> {
    DateTime::from(SystemTime::now())
}

/// Fake observe storage. Transactions work on a copy and replace the shared
/// state on commit, so concurrent transactions are not isolated: use it from
/// one task at a time.
#[derive(Default, Clone)]
pub struct InMemoryObserveStore {
    state: Arc<Mutex<State>>,
    unavailable: Arc<Mutex<bool>>,
}

impl InMemoryObserveStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// While set, `begin` fails with [`StoreError::Unavailable`].
    pub fn set_unavailable(&self, unavailable: bool) {
        *self.unavailable.lock().expect("fake lock") = unavailable;
    }

    /// Every fingerprint's call count, for tests that check nothing was double counted.
    pub fn total_calls(&self) -> u64 {
        let state = self.state.lock().expect("fake lock");
        state.fingerprints.iter().map(|f| f.calls).sum()
    }
}

#[async_trait]
impl ObserveStore for InMemoryObserveStore {
    async fn begin(&self) -> StoreResult<Box<dyn ObserveTx>> {
        if *self.unavailable.lock().expect("fake lock") {
            return Err(StoreError::Unavailable);
        }
        let work = self.state.lock().expect("fake lock").clone();
        Ok(Box::new(InMemoryTx {
            shared: self.state.clone(),
            work,
        }))
    }

    async fn purge_batches(&self, before: DateTime<Utc>) -> StoreResult<u64> {
        let mut state = self.state.lock().expect("fake lock");
        let count = state.seen.len();
        state.seen.retain(|_, seen| *seen >= before);
        Ok((count - state.seen.len()) as u64)
    }
}

#[async_trait]
impl ChangeFeed<EndpointEvent> for InMemoryObserveStore {
    async fn read(
        &self,
        after: Cursor,
        limit: usize,
    ) -> Result<Vec<Change<EndpointEvent>>, FeedError> {
        let state = self.state.lock().expect("fake lock");
        Ok(state
            .outbox
            .iter()
            .filter(|change| change.cursor > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

struct InMemoryTx {
    shared: Arc<Mutex<State>>,
    work: State,
}

#[async_trait]
impl ObserveTx for InMemoryTx {
    async fn commit(self: Box<Self>) -> StoreResult<()> {
        *self.shared.lock().expect("fake lock") = self.work;
        Ok(())
    }

    async fn lock_project(&mut self, _: ProjectId) -> StoreResult<()> {
        Ok(())
    }

    async fn first_delivery(&mut self, batch_id: Uuid) -> StoreResult<bool> {
        Ok(self.work.seen.insert(batch_id, now()).is_none())
    }

    async fn address(
        &mut self,
        project: ProjectId,
        address: &ServiceAddress,
    ) -> StoreResult<Option<AddressRow>> {
        Ok(self
            .work
            .addresses
            .get(&(project, address.clone()))
            .cloned())
    }

    async fn addresses(&mut self, project: ProjectId) -> StoreResult<Vec<AddressRow>> {
        let mut rows: Vec<_> = self
            .work
            .addresses
            .iter()
            .filter(|((owner, _), _)| *owner == project)
            .map(|(_, row)| row.clone())
            .collect();
        rows.sort_by(|a, b| a.address.cmp(&b.address));
        Ok(rows)
    }

    async fn count_address(
        &mut self,
        project: ProjectId,
        address: &ServiceAddress,
        at: DateTime<Utc>,
    ) -> StoreResult<()> {
        let row = self
            .work
            .addresses
            .entry((project, address.clone()))
            .or_insert_with(|| AddressRow {
                address: address.clone(),
                manual: None,
                calls: 0,
                last_seen: None,
            });
        row.calls += 1;
        row.last_seen = row.last_seen.max(Some(at));
        Ok(())
    }

    async fn set_manual(
        &mut self,
        project: ProjectId,
        address: &ServiceAddress,
        verdict: Option<Verdict>,
    ) -> StoreResult<()> {
        let key = (project, address.clone());
        let row = self
            .work
            .addresses
            .entry(key.clone())
            .or_insert(AddressRow {
                address: address.clone(),
                manual: None,
                calls: 0,
                last_seen: None,
            });
        row.manual = verdict;
        if row.manual.is_none() && row.last_seen.is_none() {
            self.work.addresses.remove(&key);
        }
        Ok(())
    }

    async fn base_paths(&mut self, project: ProjectId) -> StoreResult<Vec<BasePath>> {
        Ok(self
            .work
            .base_paths
            .iter()
            .filter(|((owner, _, _), _)| *owner == project)
            .map(|((_, environment_id, address), base_path)| BasePath {
                environment_id: *environment_id,
                address: address.clone(),
                base_path: base_path.clone(),
            })
            .collect())
    }

    async fn set_base_path(&mut self, project: ProjectId, base: &BasePath) -> StoreResult<()> {
        self.work.base_paths.insert(
            (project, base.environment_id, base.address.clone()),
            base.base_path.clone(),
        );
        Ok(())
    }

    async fn find_endpoint(
        &mut self,
        project: ProjectId,
        method: &str,
        path_template: &str,
    ) -> StoreResult<Option<EndpointId>> {
        Ok(self
            .work
            .current(project)
            .find(|row| row.method == method && row.path_template == path_template)
            .map(|row| row.id))
    }

    async fn create_endpoint(&mut self, endpoint: &EndpointRow) -> StoreResult<()> {
        self.work.endpoints.push((endpoint.clone(), None));
        Ok(())
    }

    async fn endpoints(&mut self, project: ProjectId) -> StoreResult<Vec<EndpointRow>> {
        Ok(self.work.current(project).cloned().collect())
    }

    async fn declared_endpoints(&mut self, project: ProjectId) -> StoreResult<Vec<EndpointRow>> {
        let declared = &self.work.declarations;
        Ok(self
            .work
            .current(project)
            .filter(|row| declared.iter().any(|(id, _)| *id == row.id))
            .cloned()
            .collect())
    }

    async fn endpoint(&mut self, id: EndpointId) -> StoreResult<Option<EndpointRow>> {
        let Some((row, into)) = self.work.endpoints.iter().find(|(row, _)| row.id == id) else {
            return Ok(None);
        };
        let target = into.unwrap_or(row.id);
        Ok(self
            .work
            .endpoints
            .iter()
            .find(|(row, _)| row.id == target)
            .map(|(row, _)| row.clone()))
    }

    async fn aliases(&mut self, id: EndpointId) -> StoreResult<Vec<EndpointId>> {
        Ok(self
            .work
            .endpoints
            .iter()
            .filter(|(_, into)| *into == Some(id))
            .map(|(row, _)| row.id)
            .collect())
    }

    async fn alias(&mut self, from: EndpointId, into: EndpointId) -> StoreResult<()> {
        for (row, target) in &mut self.work.endpoints {
            if row.id == from || *target == Some(from) {
                *target = Some(into);
            }
        }
        Ok(())
    }

    async fn count_fingerprint(
        &mut self,
        traffic: &Traffic,
        hash: &FingerprintHash,
        at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        let found = self
            .work
            .fingerprints
            .iter_mut()
            .find(|f| f.traffic == *traffic && f.hash == *hash);
        Ok(match found {
            Some(fingerprint) => {
                fingerprint.calls += 1;
                fingerprint.first_seen = fingerprint.first_seen.min(at);
                fingerprint.last_seen = fingerprint.last_seen.max(at);
                true
            }
            None => false,
        })
    }

    async fn insert_fingerprint(&mut self, fingerprint: &NewFingerprint) -> StoreResult<()> {
        self.work.fingerprints.push(StoredFingerprint {
            traffic: fingerprint.traffic.clone(),
            hash: fingerprint.hash,
            structure: fingerprint.structure.clone(),
            sample: fingerprint.sample.clone(),
            calls: 1,
            first_seen: fingerprint.seen,
            last_seen: fingerprint.seen,
        });
        Ok(())
    }

    async fn fingerprints(&mut self, endpoint: EndpointId) -> StoreResult<Vec<FingerprintStats>> {
        Ok(self
            .work
            .fingerprints
            .iter()
            .filter(|f| f.traffic.endpoint == endpoint)
            .map(|f| FingerprintStats {
                hash: f.hash,
                environment_id: f.traffic.environment_id,
                address: f.traffic.address.clone(),
                structure: f.structure.clone(),
                calls: f.calls,
                first_seen: f.first_seen,
                last_seen: f.last_seen,
            })
            .collect())
    }

    async fn sample(
        &mut self,
        traffic: &Traffic,
        hash: &FingerprintHash,
    ) -> StoreResult<Option<Value>> {
        Ok(self
            .work
            .fingerprints
            .iter()
            .find(|f| f.traffic == *traffic && f.hash == *hash)
            .map(|f| f.sample.clone()))
    }

    async fn traffic(&mut self, project: ProjectId) -> StoreResult<Vec<Traffic>> {
        let mut groups: Vec<Traffic> = Vec::new();
        for fingerprint in &self.work.fingerprints {
            if self.work.project_of(fingerprint.traffic.endpoint) == Some(project)
                && !groups.contains(&fingerprint.traffic)
            {
                groups.push(fingerprint.traffic.clone());
            }
        }
        Ok(groups)
    }

    async fn usage(
        &mut self,
        project: ProjectId,
    ) -> StoreResult<Vec<(EndpointId, EnvironmentUsage)>> {
        let mut usage: Vec<(EndpointId, EnvironmentUsage)> = Vec::new();
        for f in &self.work.fingerprints {
            if self.work.project_of(f.traffic.endpoint) != Some(project) {
                continue;
            }
            let key = (f.traffic.endpoint, f.traffic.environment_id);
            match usage
                .iter_mut()
                .find(|(id, u)| (*id, u.environment_id) == key)
            {
                Some((_, u)) => {
                    u.calls += f.calls;
                    u.first_seen = u.first_seen.min(f.first_seen);
                    u.last_seen = u.last_seen.max(f.last_seen);
                }
                None => usage.push((
                    key.0,
                    EnvironmentUsage {
                        environment_id: key.1,
                        calls: f.calls,
                        first_seen: f.first_seen,
                        last_seen: f.last_seen,
                    },
                )),
            }
        }
        Ok(usage)
    }

    async fn move_traffic(&mut self, from: &Traffic, to: EndpointId) -> StoreResult<()> {
        let target = Traffic {
            endpoint: to,
            ..from.clone()
        };
        let (moving, staying): (Vec<_>, Vec<_>) = std::mem::take(&mut self.work.fingerprints)
            .into_iter()
            .partition(|f| f.traffic == *from);
        self.work.fingerprints = staying;
        for mut fingerprint in moving {
            match self
                .work
                .fingerprints
                .iter_mut()
                .find(|f| f.traffic == target && f.hash == fingerprint.hash)
            {
                Some(existing) => {
                    existing.calls += fingerprint.calls;
                    existing.first_seen = existing.first_seen.min(fingerprint.first_seen);
                    existing.last_seen = existing.last_seen.max(fingerprint.last_seen);
                }
                None => {
                    fingerprint.traffic = target.clone();
                    self.work.fingerprints.push(fingerprint);
                }
            }
        }
        Ok(())
    }

    async fn put_declaration(
        &mut self,
        endpoint: EndpointId,
        declaration: &Declaration,
    ) -> StoreResult<bool> {
        let same_source = |(id, stored): &&mut (EndpointId, Declaration)| {
            *id == endpoint
                && stored.environment_id == declaration.environment_id
                && stored.platform == declaration.platform
                && stored.source_url == declaration.source_url
        };
        match self.work.declarations.iter_mut().find(same_source) {
            Some((_, stored)) => {
                if stored.updated_at > declaration.updated_at
                    || stored.structure == declaration.structure
                {
                    return Ok(false);
                }
                *stored = declaration.clone();
            }
            None => self.work.declarations.push((endpoint, declaration.clone())),
        }
        Ok(true)
    }

    async fn declarations(&mut self, endpoint: EndpointId) -> StoreResult<Vec<Declaration>> {
        Ok(self
            .work
            .declarations
            .iter()
            .filter(|(id, _)| *id == endpoint)
            .map(|(_, declaration)| declaration.clone())
            .collect())
    }

    async fn publish(&mut self, events: &[EndpointEvent]) -> StoreResult<()> {
        for event in events {
            let cursor = Cursor(self.work.outbox.len() as i64 + 1);
            self.work.outbox.push(Change {
                cursor,
                at: now(),
                event: event.clone(),
            });
        }
        Ok(())
    }
}

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_790_000_000 + seconds, 0).expect("valid timestamp")
}

/// Behaviour every [`ObserveStore`] must show. Uses fresh IDs, so it can run
/// against a shared database, but nothing else may publish meanwhile.
/// Panics on the first violation.
pub async fn observe_store_conformance<S>(store: &S)
where
    S: ObserveStore + ChangeFeed<EndpointEvent>,
{
    let project = ProjectId::new();
    let environment = EnvironmentId::new();
    let api = ServiceAddress::parse("https://api.example.com").expect("address");
    let begin = || async { store.begin().await.expect("begin") };

    // Batches: recorded once; an uncommitted transaction leaves no trace.
    let batch = Uuid::new_v4();
    let mut tx = begin().await;
    tx.lock_project(project).await.expect("lock");
    assert!(tx.first_delivery(batch).await.unwrap());
    drop(tx);
    let mut tx = begin().await;
    assert!(tx.first_delivery(batch).await.unwrap(), "rolled back");
    assert!(!tx.first_delivery(batch).await.unwrap(), "same transaction");
    tx.commit().await.unwrap();
    let mut tx = begin().await;
    assert!(!tx.first_delivery(batch).await.unwrap(), "committed");
    drop(tx);
    assert!(
        store.purge_batches(at(0)).await.unwrap() == 0,
        "recent batches stay"
    );

    // Addresses.
    let analytics = ServiceAddress::parse("https://analytics.example.net").expect("address");
    let mut tx = begin().await;
    assert_eq!(tx.address(project, &api).await.unwrap(), None);
    tx.count_address(project, &api, at(10)).await.unwrap();
    tx.count_address(project, &api, at(5)).await.unwrap();
    let row = tx.address(project, &api).await.unwrap().expect("counted");
    assert_eq!(
        (row.manual, row.calls, row.last_seen),
        (None, 2, Some(at(10))),
        "traffic alone decides nothing"
    );
    assert_eq!(row.verdict(), Verdict::Own);
    tx.set_manual(project, &api, Some(Verdict::External))
        .await
        .unwrap();
    assert_eq!(
        tx.address(project, &api).await.unwrap().unwrap().verdict(),
        Verdict::External
    );
    tx.set_manual(project, &api, None).await.unwrap();
    assert_eq!(
        tx.address(project, &api).await.unwrap().unwrap().verdict(),
        Verdict::Own,
        "an address with traffic stays"
    );
    tx.set_manual(project, &analytics, Some(Verdict::External))
        .await
        .unwrap();
    let listed = tx.addresses(project).await.unwrap();
    assert_eq!(
        listed.iter().map(|r| &r.address).collect::<Vec<_>>(),
        [&analytics, &api],
        "ordered by address"
    );
    assert_eq!((listed[0].calls, listed[0].last_seen), (0, None));
    tx.set_manual(project, &analytics, None).await.unwrap();
    assert_eq!(tx.address(project, &analytics).await.unwrap(), None);
    let other = ProjectId::new();
    tx.count_address(other, &api, at(1)).await.unwrap();
    assert_eq!(
        tx.address(project, &api).await.unwrap().unwrap().calls,
        2,
        "counted per project"
    );

    // Base paths.
    let base = BasePath {
        environment_id: environment,
        address: api.clone(),
        base_path: "/api".into(),
    };
    tx.set_base_path(project, &base).await.unwrap();
    assert_eq!(tx.base_paths(project).await.unwrap(), vec![base]);
    assert!(tx.base_paths(other).await.unwrap().is_empty());

    // Endpoints and aliases.
    let endpoint = |template: &str| EndpointRow {
        id: EndpointId::new(),
        project_id: project,
        method: "GET".into(),
        path_template: template.into(),
    };
    let (a, b, c) = (endpoint("/a"), endpoint("/b"), endpoint("/c"));
    for row in [&a, &b, &c] {
        tx.create_endpoint(row).await.unwrap();
    }
    assert_eq!(
        tx.find_endpoint(project, "GET", "/a").await.unwrap(),
        Some(a.id)
    );
    assert_eq!(tx.find_endpoint(project, "POST", "/a").await.unwrap(), None);
    assert_eq!(tx.find_endpoint(other, "GET", "/a").await.unwrap(), None);
    tx.alias(a.id, b.id).await.unwrap();
    tx.alias(b.id, c.id).await.unwrap();
    assert_eq!(tx.endpoint(a.id).await.unwrap(), Some(c.clone()));
    assert_eq!(tx.endpoint(EndpointId::new()).await.unwrap(), None);
    let mut aliases = tx.aliases(c.id).await.unwrap();
    aliases.sort_by_key(|id| id.to_string());
    let mut expected = vec![a.id, b.id];
    expected.sort_by_key(|id| id.to_string());
    assert_eq!(aliases, expected, "aliases follow a second merge");
    assert_eq!(tx.endpoints(project).await.unwrap(), vec![c.clone()]);
    assert_eq!(tx.find_endpoint(project, "GET", "/a").await.unwrap(), None);
    let d = endpoint("/d");
    tx.create_endpoint(&d).await.unwrap();

    // Fingerprints.
    let traffic = |endpoint: &EndpointRow| Traffic {
        endpoint: endpoint.id,
        environment_id: environment,
        address: api.clone(),
    };
    let fingerprint = |endpoint: &EndpointRow, hash: u8, seen| NewFingerprint {
        traffic: traffic(endpoint),
        hash: [hash; 32],
        structure: json!({ "hash": hash }),
        sample: json!({ "sample": hash }),
        seen,
    };
    assert!(
        !tx.count_fingerprint(&traffic(&c), &[1; 32], at(0))
            .await
            .unwrap()
    );
    tx.insert_fingerprint(&fingerprint(&c, 1, at(20)))
        .await
        .unwrap();
    assert!(
        tx.count_fingerprint(&traffic(&c), &[1; 32], at(10))
            .await
            .unwrap()
    );
    tx.insert_fingerprint(&fingerprint(&d, 1, at(30)))
        .await
        .unwrap();
    tx.insert_fingerprint(&fingerprint(&d, 2, at(40)))
        .await
        .unwrap();
    assert_eq!(tx.traffic(project).await.unwrap().len(), 2);
    tx.move_traffic(&traffic(&d), c.id).await.unwrap();
    assert!(tx.fingerprints(d.id).await.unwrap().is_empty());
    let mut moved = tx.fingerprints(c.id).await.unwrap();
    moved.sort_by_key(|f| f.calls);
    assert_eq!(moved.len(), 2, "equal fingerprints add up");
    assert_eq!(moved[0].structure, json!({ "hash": 2 }));
    assert_eq!(moved[0].hash, [2; 32]);
    assert_eq!(
        tx.sample(&traffic(&c), &[2; 32]).await.unwrap(),
        Some(json!({ "sample": 2 })),
        "samples move with their fingerprints"
    );
    assert_eq!(
        tx.sample(&traffic(&c), &[1; 32]).await.unwrap(),
        Some(json!({ "sample": 1 }))
    );
    assert_eq!(tx.sample(&traffic(&d), &[2; 32]).await.unwrap(), None);
    assert_eq!(tx.sample(&traffic(&c), &[3; 32]).await.unwrap(), None);
    assert_eq!(
        (moved[1].calls, moved[1].first_seen, moved[1].last_seen),
        (3, at(10), at(30))
    );
    assert_eq!(tx.traffic(project).await.unwrap(), vec![traffic(&c)]);
    let usage = tx.usage(project).await.unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].0, c.id);
    assert_eq!(
        (
            usage[0].1.calls,
            usage[0].1.first_seen,
            usage[0].1.last_seen
        ),
        (4, at(10), at(40))
    );

    // Declarations.
    let declaration = |structure: Value, updated_at| Declaration {
        environment_id: None,
        platform: "swagger-sync".into(),
        source_url: Some("https://docs.example.com/v3/api-docs".into()),
        structure,
        updated_at,
    };
    assert!(tx.declared_endpoints(project).await.unwrap().is_empty());
    assert!(
        tx.put_declaration(c.id, &declaration(json!(1), at(10)))
            .await
            .unwrap()
    );
    assert!(
        !tx.put_declaration(c.id, &declaration(json!(1), at(20)))
            .await
            .unwrap(),
        "unchanged"
    );
    assert!(
        !tx.put_declaration(c.id, &declaration(json!(2), at(5)))
            .await
            .unwrap(),
        "older"
    );
    assert!(
        tx.put_declaration(c.id, &declaration(json!(3), at(30)))
            .await
            .unwrap()
    );
    let env_specific = Declaration {
        environment_id: Some(environment),
        ..declaration(json!(4), at(30))
    };
    assert!(tx.put_declaration(c.id, &env_specific).await.unwrap());
    let mut stored = tx.declarations(c.id).await.unwrap();
    stored.sort_by_key(|d| d.environment_id.is_some());
    assert_eq!(stored, vec![declaration(json!(3), at(30)), env_specific]);
    assert_eq!(
        tx.declared_endpoints(project).await.unwrap(),
        vec![c.clone()]
    );

    // Outbox.
    let before = store
        .read(Cursor::START, usize::MAX)
        .await
        .expect("read")
        .last()
        .map_or(Cursor::START, |change| change.cursor);
    let events: Vec<_> = [a.id, b.id, c.id]
        .into_iter()
        .map(|id| EndpointEvent {
            project_id: project,
            endpoint: id,
            change: EndpointChange::StructureChanged {
                environment_id: Some(environment),
            },
        })
        .collect();
    tx.publish(&events).await.unwrap();
    assert!(
        store.read(before, usize::MAX).await.unwrap().is_empty(),
        "invisible before commit"
    );
    tx.commit().await.unwrap();
    let published: Vec<_> = store
        .read(before, usize::MAX)
        .await
        .unwrap()
        .into_iter()
        .map(|change| change.event)
        .collect();
    assert_eq!(published, events);
    nexofolio_contracts::testing::change_feed_conformance(store).await;

    // Committed state is what a new transaction sees.
    let mut tx = begin().await;
    assert_eq!(tx.endpoint(b.id).await.unwrap(), Some(c));
    drop(tx);
    let later = at(0) + TimeDelta::days(365 * 100);
    assert!(store.purge_batches(later).await.unwrap() >= 1);
    let mut tx = begin().await;
    assert!(tx.first_delivery(batch).await.unwrap(), "purged");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_passes_observe_store_conformance() {
        observe_store_conformance(&InMemoryObserveStore::new()).await;
    }
}
