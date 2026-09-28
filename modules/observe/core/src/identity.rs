//! Which endpoint a call belongs to, and moving traffic when that answer
//! changes (a late declaration, a new base path, a new address verdict).

use std::collections::HashMap;

use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::endpoint::{EndpointChange, EndpointEvent, ServiceAddress, Verdict};
use nexofolio_observe_contracts::{BasePath, EndpointRow, ObserveTx, StoreResult};

use crate::template;

/// Rehoming until nothing moves; a pass can teach a base path that moves
/// more on the next one. More than a few passes would mean a bug.
const MAX_PASSES: usize = 4;

/// What own templates depend on: base paths and declared templates.
pub struct Known {
    project: ProjectId,
    bases: Vec<BasePath>,
    declared: Vec<EndpointRow>,
}

impl Known {
    pub async fn load(tx: &mut dyn ObserveTx, project: ProjectId) -> StoreResult<Self> {
        Ok(Self {
            project,
            bases: tx.base_paths(project).await?,
            declared: tx.declared_endpoints(project).await?,
        })
    }

    fn base(&self, environment: EnvironmentId, address: &ServiceAddress) -> Option<&str> {
        self.bases
            .iter()
            .find(|base| base.environment_id == environment && base.address == *address)
            .map(|base| base.base_path.as_str())
    }

    /// The own template of `path`; `true` when it taught a new base path.
    pub async fn own_template(
        &mut self,
        tx: &mut dyn ObserveTx,
        environment: EnvironmentId,
        address: &ServiceAddress,
        method: &str,
        path: &str,
    ) -> StoreResult<(String, bool)> {
        let declared: Vec<&str> = self
            .declared
            .iter()
            .filter(|row| row.method == method)
            .map(|row| row.path_template.as_str())
            .collect();
        let own = template::own(path, self.base(environment, address), &declared);
        let Some(base_path) = own.new_base else {
            return Ok((own.template, false));
        };
        let base = BasePath {
            environment_id: environment,
            address: address.clone(),
            base_path,
        };
        tx.set_base_path(self.project, &base).await?;
        self.bases.push(base);
        Ok((own.template, true))
    }
}

/// The current endpoint with this identity, created (and announced) if new.
pub async fn ensure_endpoint(
    tx: &mut dyn ObserveTx,
    project: ProjectId,
    method: &str,
    path_template: &str,
    events: &mut Vec<EndpointEvent>,
) -> StoreResult<EndpointId> {
    if let Some(id) = tx.find_endpoint(project, method, path_template).await? {
        return Ok(id);
    }
    let row = EndpointRow {
        id: EndpointId::new(),
        project_id: project,
        method: method.to_owned(),
        path_template: path_template.to_owned(),
    };
    tx.create_endpoint(&row).await?;
    events.push(EndpointEvent {
        project_id: project,
        endpoint: row.id,
        change: EndpointChange::Created {
            method: row.method,
            path_template: row.path_template,
        },
    });
    Ok(row.id)
}

/// Recomputes every traffic group's endpoint from what is known now and
/// moves the groups whose answer changed. An endpoint left with neither
/// traffic nor declarations becomes an alias of the one that took most of
/// its calls.
pub async fn rehome(
    tx: &mut dyn ObserveTx,
    project: ProjectId,
    events: &mut Vec<EndpointEvent>,
) -> StoreResult<()> {
    for _ in 0..MAX_PASSES {
        if !pass(tx, project, events).await? {
            return Ok(());
        }
    }
    tracing::warn!(%project, "rehoming did not settle");
    Ok(())
}

async fn pass(
    tx: &mut dyn ObserveTx,
    project: ProjectId,
    events: &mut Vec<EndpointEvent>,
) -> StoreResult<bool> {
    let verdicts: HashMap<ServiceAddress, Verdict> = tx
        .addresses(project)
        .await?
        .into_iter()
        .filter_map(|row| Some((row.address.clone(), row.verdict()?)))
        .collect();
    let endpoints: HashMap<EndpointId, EndpointRow> = tx
        .endpoints(project)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    let mut known = Known::load(tx, project).await?;
    let mut changed = false;
    // Source, target and the calls moved between them.
    let mut moves: Vec<(EndpointId, EndpointId, u64)> = Vec::new();
    for traffic in tx.traffic(project).await? {
        let Some(row) = endpoints.get(&traffic.endpoint) else {
            continue;
        };
        let (environment, address) = (traffic.environment_id, &traffic.address);
        let path = template::path_of(
            &row.path_template,
            address,
            known.base(environment, address),
        );
        let target = match verdicts.get(address).copied().unwrap_or(Verdict::Own) {
            Verdict::External => template::external(address, &path),
            Verdict::Own => {
                let (target, learnt) = known
                    .own_template(tx, environment, address, &row.method, &path)
                    .await?;
                changed |= learnt;
                target
            }
        };
        if target == row.path_template {
            continue;
        }
        let calls = tx
            .fingerprints(row.id)
            .await?
            .iter()
            .filter(|f| f.environment_id == environment && f.address == *address)
            .map(|f| f.calls)
            .sum();
        let to = ensure_endpoint(tx, project, &row.method, &target, events).await?;
        tx.move_traffic(&traffic, to).await?;
        events.push(EndpointEvent {
            project_id: project,
            endpoint: to,
            change: EndpointChange::StructureChanged {
                environment_id: Some(environment),
            },
        });
        moves.push((row.id, to, calls));
        changed = true;
    }
    let mut sources: Vec<EndpointId> = Vec::new();
    for (from, _, _) in &moves {
        if !sources.contains(from) {
            sources.push(*from);
        }
    }
    for source in sources {
        if !tx.fingerprints(source).await?.is_empty() || !tx.declarations(source).await?.is_empty()
        {
            continue;
        }
        let mut received: Vec<(EndpointId, u64)> = Vec::new();
        for (_, to, calls) in moves.iter().filter(|(from, _, _)| *from == source) {
            match received.iter_mut().find(|(target, _)| target == to) {
                Some((_, total)) => *total += calls,
                None => received.push((*to, *calls)),
            }
        }
        // First of the most: stable when two targets took as many calls.
        let (into, _) =
            received.iter().fold(
                received[0],
                |best, next| if next.1 > best.1 { *next } else { best },
            );
        tx.alias(source, into).await?;
        events.push(EndpointEvent {
            project_id: project,
            endpoint: source,
            change: EndpointChange::MergedInto { into },
        });
    }
    Ok(changed)
}
