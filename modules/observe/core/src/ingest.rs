//! Turning one delivered observation into counts, fingerprints and events
//! (0003 §4.1).

use std::collections::HashMap;

use nexofolio_common::ProjectId;
use nexofolio_contracts::endpoint::{EndpointChange, EndpointEvent, ServiceAddress, Verdict};
use nexofolio_contracts::observation::{CanonicalObservation, Fact, HttpDeclaration, HttpExchange};
use nexofolio_observe_contracts::{
    Declaration, NewFingerprint, ObserveTx, StoreError, StoreResult, Traffic,
};

use crate::declaration::DeclaredFields;
use crate::identity::{Known, ensure_endpoint};
use crate::structure::{Structure, path_of};
use crate::{address, template};

enum Failure {
    Store(StoreError),
    /// Only this observation is dropped; the reason goes to the log.
    Skip(&'static str),
}

impl From<StoreError> for Failure {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// One transaction's worth of observations.
pub struct Ingest<'a> {
    tx: &'a mut dyn ObserveTx,
    known: HashMap<ProjectId, Known>,
    events: Vec<EndpointEvent>,
    rehome: Vec<ProjectId>,
}

impl<'a> Ingest<'a> {
    pub fn new(tx: &'a mut dyn ObserveTx) -> Self {
        Self {
            tx,
            known: HashMap::new(),
            events: Vec::new(),
            rehome: Vec::new(),
        }
    }

    /// Events so far, and projects whose identities need rehoming.
    pub fn finish(self) -> (Vec<EndpointEvent>, Vec<ProjectId>) {
        (self.events, self.rehome)
    }

    pub async fn observe(&mut self, observation: &CanonicalObservation) -> StoreResult<()> {
        let result = match &observation.fact {
            Fact::Exchange(exchange) => self.exchange(observation, exchange).await,
            Fact::Declaration(declaration) => self.declaration(observation, declaration).await,
        };
        match result {
            Ok(()) => Ok(()),
            Err(Failure::Store(error)) => Err(error),
            Err(Failure::Skip(reason)) => {
                tracing::warn!(
                    batch_id = %observation.key.batch_id,
                    record_id = %observation.key.record_id,
                    reason,
                    "observation skipped"
                );
                Ok(())
            }
        }
    }

    async fn exchange(
        &mut self,
        observation: &CanonicalObservation,
        exchange: &HttpExchange,
    ) -> Result<(), Failure> {
        let project = observation.project_id;
        let environment = observation
            .environment_id
            .ok_or(Failure::Skip("exchange without environment"))?;
        let url = &exchange.request.url;
        let address = ServiceAddress::of_url(url)
            .ok_or(Failure::Skip("request url has no service address"))?;
        let method = exchange.request.method.to_ascii_uppercase();

        let row = self.tx.address(project, &address).await?;
        let auto = match row.as_ref().and_then(|row| row.auto) {
            Some(auto) => auto,
            None => {
                let others = self.tx.other_projects_using(project, &address).await?;
                address::auto_verdict(&address, observation, others)
            }
        };
        self.tx
            .count_address(project, &address, auto, observation.observed_at)
            .await?;
        let verdict = row.and_then(|row| row.manual).unwrap_or(auto.verdict);

        let path = path_of(url);
        let path_template = match verdict {
            Verdict::External => template::external(&address, path),
            Verdict::Own => {
                let mut known = match self.known.remove(&project) {
                    Some(known) => known,
                    None => Known::load(self.tx, project).await?,
                };
                let result = known
                    .own_template(self.tx, environment, &address, &method, path)
                    .await;
                self.known.insert(project, known);
                let (template, learnt) = result?;
                if learnt {
                    self.needs_rehome(project);
                }
                template
            }
        };
        let endpoint =
            ensure_endpoint(self.tx, project, &method, &path_template, &mut self.events).await?;

        let structure = Structure::of(exchange);
        let hash = structure.hash();
        let traffic = Traffic {
            endpoint,
            environment_id: environment,
            address,
        };
        if self
            .tx
            .count_fingerprint(&traffic, &hash, observation.observed_at)
            .await?
        {
            return Ok(());
        }
        let fingerprint = NewFingerprint {
            traffic,
            hash,
            structure: serde_json::to_value(&structure).expect("structures serialise"),
            sample: serde_json::to_value(observation)
                .map_err(|_| Failure::Skip("observation does not serialise"))?,
            seen: observation.observed_at,
        };
        self.tx.insert_fingerprint(&fingerprint).await?;
        self.events.push(EndpointEvent {
            project_id: project,
            endpoint,
            change: EndpointChange::StructureChanged {
                environment_id: Some(environment),
            },
        });
        Ok(())
    }

    async fn declaration(
        &mut self,
        observation: &CanonicalObservation,
        declaration: &HttpDeclaration,
    ) -> Result<(), Failure> {
        let project = observation.project_id;
        let path_template = template::declared(&declaration.path_template)
            .ok_or(Failure::Skip("declared path template is not a path"))?;
        let method = declaration.method.to_ascii_uppercase();
        let endpoint =
            ensure_endpoint(self.tx, project, &method, &path_template, &mut self.events).await?;
        let newly_declared = self.tx.declarations(endpoint).await?.is_empty();
        let stored = Declaration {
            environment_id: observation.environment_id,
            platform: observation.source.platform.clone(),
            source_url: observation.source.source_url.clone(),
            structure: serde_json::to_value(DeclaredFields::of(declaration))
                .expect("declared fields serialise"),
            updated_at: observation.observed_at,
        };
        if self.tx.put_declaration(endpoint, &stored).await? {
            self.events.push(EndpointEvent {
                project_id: project,
                endpoint,
                change: EndpointChange::StructureChanged {
                    environment_id: observation.environment_id,
                },
            });
        }
        if newly_declared {
            // Later calls must see the new template.
            self.known.remove(&project);
            self.needs_rehome(project);
        }
        Ok(())
    }

    fn needs_rehome(&mut self, project: ProjectId) {
        if !self.rehome.contains(&project) {
            self.rehome.push(project);
        }
    }
}
