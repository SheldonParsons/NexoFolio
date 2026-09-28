//! Observe: turns observations into endpoints, fingerprints and field facts
//! (0003 §4).
//!
//! Everything is decided here; the [`ObserveStore`] port only remembers.
//! [`Observe`] is the whole public surface: it is the [`ObservationSink`]
//! intake delivers to, the [`EndpointReader`] and the [`ServiceAddresses`].
//! Its change feed is served by the store directly.
//!
//! One delivery is one store transaction: batches seen before are skipped,
//! so a redelivered batch counts once, and events are published right before
//! commit. A bad observation is skipped with a warning; only storage failures
//! fail the delivery.

mod address;
mod declaration;
mod facts;
mod identity;
mod ingest;
mod shape;
mod structure;
mod template;

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::endpoint::{
    AddressStatus, AddressUse, Decision, EndpointError, EndpointEvent, EndpointFacts,
    EndpointReader, EndpointSummary, EnvironmentUsage, ServiceAddress, ServiceAddresses, Verdict,
};
use nexofolio_contracts::observation::{CanonicalObservation, ObservationSink, SinkError};
use nexofolio_observe_contracts::{ObserveStore, ObserveTx, StoreError, StoreResult};

use crate::declaration::DeclaredFields;
use crate::facts::Seen;
use crate::ingest::Ingest;
use crate::structure::Structure;

/// How long a delivered batch is remembered for skipping redeliveries.
/// Matches intake's receipt retention.
pub const RETENTION: TimeDelta = TimeDelta::days(7);

pub struct Observe<S> {
    store: S,
}

/// Forgets delivered batches older than [`RETENTION`]; run periodically by
/// the worker.
pub async fn purge_expired(store: &dyn ObserveStore, now: DateTime<Utc>) -> StoreResult<u64> {
    store.purge_batches(now - RETENTION).await
}

impl<S: ObserveStore> Observe<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    async fn ingest(&self, observations: &[CanonicalObservation]) -> StoreResult<()> {
        let mut tx = self.store.begin().await?;
        let mut projects: Vec<ProjectId> = Vec::new();
        for observation in observations {
            if !projects.contains(&observation.project_id) {
                projects.push(observation.project_id);
            }
        }
        // One lock order for every writer.
        projects.sort_by_key(ProjectId::to_string);
        for project in &projects {
            tx.lock_project(*project).await?;
        }
        let mut batches = HashSet::new();
        let mut fresh = HashSet::new();
        for observation in observations {
            let batch = observation.key.batch_id;
            if batches.insert(batch) && tx.first_delivery(batch).await? {
                fresh.insert(batch);
            }
        }
        let mut ingest = Ingest::new(tx.as_mut());
        for observation in observations {
            if fresh.contains(&observation.key.batch_id) {
                ingest.observe(observation).await?;
            }
        }
        let (mut events, rehome) = ingest.finish();
        for project in rehome {
            identity::rehome(tx.as_mut(), project, &mut events).await?;
        }
        finish(tx, events).await
    }

    async fn read(&self) -> Result<Box<dyn ObserveTx>, EndpointError> {
        self.store.begin().await.map_err(unavailable)
    }
}

/// Publishes the events once each, then commits.
async fn finish(mut tx: Box<dyn ObserveTx>, events: Vec<EndpointEvent>) -> StoreResult<()> {
    let mut unique: Vec<EndpointEvent> = Vec::new();
    for event in events {
        if !unique.contains(&event) {
            unique.push(event);
        }
    }
    if !unique.is_empty() {
        tx.publish(&unique).await?;
    }
    tx.commit().await
}

fn unavailable(_: StoreError) -> EndpointError {
    EndpointError::Unavailable
}

#[async_trait]
impl<S: ObserveStore> ObservationSink for Observe<S> {
    async fn accept(&self, observations: Vec<CanonicalObservation>) -> Result<(), SinkError> {
        if observations.is_empty() {
            return Ok(());
        }
        self.ingest(&observations)
            .await
            .map_err(|_| SinkError::Unavailable)
    }
}

#[async_trait]
impl<S: ObserveStore> EndpointReader for Observe<S> {
    async fn list(&self, project: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        let mut tx = self.read().await?;
        let endpoints = tx.endpoints(project).await.map_err(unavailable)?;
        let usage = tx.usage(project).await.map_err(unavailable)?;
        let declared: HashSet<EndpointId> = tx
            .declared_endpoints(project)
            .await
            .map_err(unavailable)?
            .into_iter()
            .map(|row| row.id)
            .collect();
        let mut summaries: Vec<EndpointSummary> = endpoints
            .into_iter()
            .map(|row| {
                let mut environments: Vec<EnvironmentUsage> = usage
                    .iter()
                    .filter(|(id, _)| *id == row.id)
                    .map(|(_, usage)| usage.clone())
                    .collect();
                environments.sort_by_key(|usage| usage.environment_id.to_string());
                EndpointSummary {
                    external: !row.path_template.starts_with('/'),
                    declared: declared.contains(&row.id),
                    id: row.id,
                    project_id: row.project_id,
                    method: row.method,
                    path_template: row.path_template,
                    environments,
                }
            })
            .collect();
        summaries.sort_by(|a, b| (&a.path_template, &a.method).cmp(&(&b.path_template, &b.method)));
        Ok(summaries)
    }

    async fn get(&self, id: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        let mut tx = self.read().await?;
        let Some(row) = tx.endpoint(id).await.map_err(unavailable)? else {
            return Ok(None);
        };
        let aliases = tx.aliases(row.id).await.map_err(unavailable)?;
        let fingerprints = tx.fingerprints(row.id).await.map_err(unavailable)?;
        let declarations = tx.declarations(row.id).await.map_err(unavailable)?;
        let bases = tx.base_paths(row.project_id).await.map_err(unavailable)?;

        let mut environments: Vec<EnvironmentUsage> = Vec::new();
        let mut addresses: Vec<AddressUse> = Vec::new();
        let mut seen: Vec<Seen> = Vec::new();
        for fingerprint in fingerprints {
            match environments
                .iter_mut()
                .find(|usage| usage.environment_id == fingerprint.environment_id)
            {
                Some(usage) => {
                    usage.calls += fingerprint.calls;
                    usage.first_seen = usage.first_seen.min(fingerprint.first_seen);
                    usage.last_seen = usage.last_seen.max(fingerprint.last_seen);
                }
                None => environments.push(EnvironmentUsage {
                    environment_id: fingerprint.environment_id,
                    calls: fingerprint.calls,
                    first_seen: fingerprint.first_seen,
                    last_seen: fingerprint.last_seen,
                }),
            }
            let used = AddressUse {
                environment_id: fingerprint.environment_id,
                address: fingerprint.address.clone(),
                base_path: bases
                    .iter()
                    .find(|base| {
                        base.environment_id == fingerprint.environment_id
                            && base.address == fingerprint.address
                    })
                    .map(|base| base.base_path.clone())
                    .unwrap_or_default(),
            };
            if !addresses.contains(&used) {
                addresses.push(used);
            }
            match serde_json::from_value::<Structure>(fingerprint.structure) {
                Ok(structure) => seen.push(Seen {
                    environment_id: fingerprint.environment_id,
                    structure,
                    calls: fingerprint.calls,
                    first_seen: fingerprint.first_seen,
                    last_seen: fingerprint.last_seen,
                }),
                Err(_) => tracing::warn!(endpoint = %row.id, "unreadable fingerprint structure"),
            }
        }
        environments.sort_by_key(|usage| usage.environment_id.to_string());
        addresses.sort_by(|a, b| {
            (a.environment_id.to_string(), &a.address)
                .cmp(&(b.environment_id.to_string(), &b.address))
        });

        let declared_fields = declarations
            .iter()
            .filter_map(|declaration| {
                let fields =
                    serde_json::from_value::<DeclaredFields>(declaration.structure.clone());
                if fields.is_err() {
                    tracing::warn!(endpoint = %row.id, "unreadable declaration");
                }
                Some((declaration.updated_at, fields.ok()?))
            })
            .collect();
        let fields = facts::fields(
            &row.path_template,
            &seen,
            &declaration::merge(declared_fields),
        );

        Ok(Some(EndpointFacts {
            summary: EndpointSummary {
                external: !row.path_template.starts_with('/'),
                declared: !declarations.is_empty(),
                id: row.id,
                project_id: row.project_id,
                method: row.method,
                path_template: row.path_template,
                environments,
            },
            aliases,
            addresses,
            fields,
        }))
    }
}

#[async_trait]
impl<S: ObserveStore> ServiceAddresses for Observe<S> {
    async fn list(&self, project: ProjectId) -> Result<Vec<AddressStatus>, EndpointError> {
        let mut tx = self.read().await?;
        let rows = tx.addresses(project).await.map_err(unavailable)?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let (verdict, decision) = match (row.manual, row.auto) {
                    (Some(manual), _) => (manual, Decision::Manual),
                    (None, Some(auto)) => (auto.verdict, Decision::Auto(auto.reason)),
                    (None, None) => return None,
                };
                Some(AddressStatus {
                    address: row.address,
                    verdict,
                    decision,
                    calls: row.calls,
                    last_seen: row.last_seen,
                })
            })
            .collect())
    }

    async fn decide(
        &self,
        project: ProjectId,
        address: ServiceAddress,
        verdict: Option<Verdict>,
    ) -> Result<(), EndpointError> {
        let decided = async {
            let mut tx = self.store.begin().await?;
            tx.lock_project(project).await?;
            let before = tx
                .address(project, &address)
                .await?
                .and_then(|r| r.verdict());
            tx.set_manual(project, &address, verdict).await?;
            let after = tx
                .address(project, &address)
                .await?
                .and_then(|r| r.verdict());
            let mut events = Vec::new();
            if before != after {
                identity::rehome(tx.as_mut(), project, &mut events).await?;
            }
            finish(tx, events).await
        };
        decided.await.map_err(unavailable)
    }
}
