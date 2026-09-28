use std::fmt::Display;
use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{
    DatabaseProbe, EndpointId, EnvironmentId, Error, ProjectId, Result, Secret,
};
use nexofolio_contracts::endpoint::{EndpointEvent, EnvironmentUsage, ServiceAddress, Verdict};
use nexofolio_contracts::feed::{Change, ChangeFeed, Cursor, FeedError};
use nexofolio_observe_contracts::{
    AddressRow, AutoVerdict, BasePath, Declaration, EndpointRow, FingerprintHash, FingerprintStats,
    NewFingerprint, ObserveStore, ObserveTx, StoreError, StoreResult, Traffic,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgRow};
use sqlx::{ConnectOptions, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

/// Everything observe stores lives in this schema. Connections put it alone on
/// the `search_path`, so SQL in this crate stays unqualified.
const SCHEMA: &str = "observe";

/// Advisory lock keys (first half): one per project, one for the outbox.
const PROJECT_LOCK: i32 = 0x6f62_0001;
const OUTBOX_LOCK: i32 = 0x6f62_0002;

#[derive(Clone)]
pub struct PostgresObserve {
    pool: PgPool,
}

impl PostgresObserve {
    /// Lazy connections keep liveness available during database outages.
    pub fn new(url: &Secret, max_connections: u32, timeout: Duration) -> Result<Self> {
        if max_connections == 0 || timeout.is_zero() {
            return Err(Error::invalid("database pool limits must be positive"));
        }
        if !url.expose().starts_with("postgres://") && !url.expose().starts_with("postgresql://") {
            return Err(Error::invalid("DATABASE_URL must be a PostgreSQL URL"));
        }
        let options = PgConnectOptions::from_str(url.expose())
            .map_err(|_| Error::invalid("DATABASE_URL is invalid"))?
            .options([("search_path", SCHEMA)])
            .disable_statement_logging();
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(timeout)
            .connect_lazy_with(options);
        Ok(Self { pool })
    }

    /// Explicit administration only; API and worker never migrate automatically.
    pub async fn migrate(&self) -> Result<()> {
        let mut migrator = sqlx::migrate!("./migrations");
        migrator.create_schema(SCHEMA);
        migrator.dangerous_set_table_name(format!("{SCHEMA}._sqlx_migrations"));
        migrator
            .run(&self.pool)
            .await
            .map_err(|_| Error::Unavailable {
                component: "database_migrations",
            })
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[async_trait]
impl DatabaseProbe for PostgresObserve {
    async fn check(&self) -> Result<()> {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map(|_| ())
            .map_err(|_| Error::Unavailable {
                component: "database",
            })
    }
}

fn unavailable<E>(_: E) -> StoreError {
    StoreError::Unavailable
}

/// IDs are UUIDs underneath but do not expose them.
fn uuid(id: impl Display) -> Uuid {
    Uuid::parse_str(&id.to_string()).expect("identifiers are UUIDs")
}

fn id<T: FromStr>(uuid: Uuid) -> T {
    T::from_str(&uuid.to_string())
        .ok()
        .expect("identifiers are UUIDs")
}

/// Unit enums are stored as their serde name.
fn text<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => name,
        _ => unreachable!("stored enums serialise to strings"),
    }
}

fn parse<T: DeserializeOwned>(name: String) -> StoreResult<T> {
    serde_json::from_value(Value::String(name)).map_err(unavailable)
}

fn address(name: String) -> StoreResult<ServiceAddress> {
    ServiceAddress::parse(&name).ok_or(StoreError::Unavailable)
}

fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

#[async_trait]
impl ObserveStore for PostgresObserve {
    async fn begin(&self) -> StoreResult<Box<dyn ObserveTx>> {
        let tx = self.pool.begin().await.map_err(unavailable)?;
        Ok(Box::new(PostgresTx { tx }))
    }

    async fn purge_batches(&self, before: DateTime<Utc>) -> StoreResult<u64> {
        sqlx::query("DELETE FROM seen_batches WHERE seen_at<$1")
            .bind(before)
            .execute(&self.pool)
            .await
            .map(|done| done.rows_affected())
            .map_err(unavailable)
    }
}

#[async_trait]
impl ChangeFeed<EndpointEvent> for PostgresObserve {
    async fn read(
        &self,
        after: Cursor,
        limit: usize,
    ) -> std::result::Result<Vec<Change<EndpointEvent>>, FeedError> {
        let rows = sqlx::query(
            "SELECT seq,created_at,event FROM outbox WHERE seq>$1 ORDER BY seq LIMIT $2",
        )
        .bind(after.0)
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await
        .map_err(|_| FeedError::Unavailable)?;
        rows.into_iter()
            .map(|row| {
                let event: Value = row.try_get("event")?;
                Ok(Change {
                    cursor: Cursor(row.try_get("seq")?),
                    at: row.try_get("created_at")?,
                    event: serde_json::from_value(event)
                        .map_err(|error| sqlx::Error::Decode(Box::new(error)))?,
                })
            })
            .collect::<std::result::Result<_, sqlx::Error>>()
            .map_err(|_| FeedError::Unavailable)
    }
}

struct PostgresTx {
    tx: Transaction<'static, Postgres>,
}

fn address_row(row: &PgRow) -> StoreResult<AddressRow> {
    let auto = match (
        row.try_get::<Option<String>, _>("auto_verdict")
            .map_err(unavailable)?,
        row.try_get::<Option<String>, _>("auto_reason")
            .map_err(unavailable)?,
    ) {
        (Some(verdict), Some(reason)) => Some(AutoVerdict {
            verdict: parse(verdict)?,
            reason: parse(reason)?,
        }),
        _ => None,
    };
    let manual: Option<String> = row.try_get("manual_verdict").map_err(unavailable)?;
    Ok(AddressRow {
        address: address(row.try_get("address").map_err(unavailable)?)?,
        manual: manual.map(parse).transpose()?,
        auto,
        calls: count(row.try_get("calls").map_err(unavailable)?),
        last_seen: row.try_get("last_seen").map_err(unavailable)?,
    })
}

fn endpoint_row(row: &PgRow) -> StoreResult<EndpointRow> {
    Ok(EndpointRow {
        id: id(row.try_get("id").map_err(unavailable)?),
        project_id: id(row.try_get("project_id").map_err(unavailable)?),
        method: row.try_get("method").map_err(unavailable)?,
        path_template: row.try_get("path_template").map_err(unavailable)?,
    })
}

// Macros rather than consts: sqlx only takes SQL that is a literal.
macro_rules! address_columns {
    () => {
        "address,manual_verdict,auto_verdict,auto_reason,calls,last_seen"
    };
}
macro_rules! endpoint_columns {
    () => {
        "e.id,e.project_id,e.method,e.path_template"
    };
}

#[async_trait]
impl ObserveTx for PostgresTx {
    async fn commit(self: Box<Self>) -> StoreResult<()> {
        self.tx.commit().await.map_err(unavailable)
    }

    async fn lock_project(&mut self, project: ProjectId) -> StoreResult<()> {
        sqlx::query("SELECT pg_advisory_xact_lock($1,hashtext($2))")
            .bind(PROJECT_LOCK)
            .bind(project.to_string())
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }

    async fn first_delivery(&mut self, batch_id: Uuid) -> StoreResult<bool> {
        sqlx::query("INSERT INTO seen_batches(batch_id) VALUES($1) ON CONFLICT DO NOTHING")
            .bind(batch_id)
            .execute(&mut *self.tx)
            .await
            .map(|done| done.rows_affected() == 1)
            .map_err(unavailable)
    }

    async fn address(
        &mut self,
        project: ProjectId,
        address: &ServiceAddress,
    ) -> StoreResult<Option<AddressRow>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            address_columns!(),
            " FROM service_addresses WHERE project_id=$1 AND address=$2"
        ))
        .bind(uuid(project))
        .bind(address.as_str())
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        row.as_ref().map(address_row).transpose()
    }

    async fn addresses(&mut self, project: ProjectId) -> StoreResult<Vec<AddressRow>> {
        sqlx::query(concat!(
            "SELECT ",
            address_columns!(),
            " FROM service_addresses WHERE project_id=$1 ORDER BY address COLLATE \"C\""
        ))
        .bind(uuid(project))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?
        .iter()
        .map(address_row)
        .collect()
    }

    async fn other_projects_using(
        &mut self,
        project: ProjectId,
        address: &ServiceAddress,
    ) -> StoreResult<u64> {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM service_addresses WHERE address=$2 AND project_id<>$1 AND calls>0",
        )
        .bind(uuid(project))
        .bind(address.as_str())
        .fetch_one(&mut *self.tx)
        .await
        .map(count)
        .map_err(unavailable)
    }

    async fn count_address(
        &mut self,
        project: ProjectId,
        address: &ServiceAddress,
        auto: AutoVerdict,
        at: DateTime<Utc>,
    ) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO service_addresses(project_id,address,auto_verdict,auto_reason,calls,last_seen) \
             VALUES($1,$2,$3,$4,1,$5) ON CONFLICT(project_id,address) DO UPDATE SET \
             auto_verdict=coalesce(service_addresses.auto_verdict,excluded.auto_verdict), \
             auto_reason=coalesce(service_addresses.auto_reason,excluded.auto_reason), \
             calls=service_addresses.calls+1, \
             last_seen=greatest(service_addresses.last_seen,excluded.last_seen)",
        )
        .bind(uuid(project))
        .bind(address.as_str())
        .bind(text(auto.verdict))
        .bind(text(auto.reason))
        .bind(at)
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    async fn set_manual(
        &mut self,
        project: ProjectId,
        address: &ServiceAddress,
        verdict: Option<Verdict>,
    ) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO service_addresses(project_id,address,manual_verdict) VALUES($1,$2,$3) \
             ON CONFLICT(project_id,address) DO UPDATE SET manual_verdict=excluded.manual_verdict",
        )
        .bind(uuid(project))
        .bind(address.as_str())
        .bind(verdict.map(text))
        .execute(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        sqlx::query(
            "DELETE FROM service_addresses WHERE project_id=$1 AND address=$2 \
             AND manual_verdict IS NULL AND auto_verdict IS NULL",
        )
        .bind(uuid(project))
        .bind(address.as_str())
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    async fn base_paths(&mut self, project: ProjectId) -> StoreResult<Vec<BasePath>> {
        sqlx::query("SELECT environment_id,address,base_path FROM base_paths WHERE project_id=$1")
            .bind(uuid(project))
            .fetch_all(&mut *self.tx)
            .await
            .map_err(unavailable)?
            .iter()
            .map(|row| {
                Ok(BasePath {
                    environment_id: id::<EnvironmentId>(
                        row.try_get("environment_id").map_err(unavailable)?,
                    ),
                    address: address(row.try_get("address").map_err(unavailable)?)?,
                    base_path: row.try_get("base_path").map_err(unavailable)?,
                })
            })
            .collect()
    }

    async fn set_base_path(&mut self, project: ProjectId, base: &BasePath) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO base_paths(project_id,environment_id,address,base_path) VALUES($1,$2,$3,$4) \
             ON CONFLICT(project_id,environment_id,address) DO UPDATE SET base_path=excluded.base_path",
        )
        .bind(uuid(project))
        .bind(uuid(base.environment_id))
        .bind(base.address.as_str())
        .bind(&base.base_path)
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    async fn find_endpoint(
        &mut self,
        project: ProjectId,
        method: &str,
        path_template: &str,
    ) -> StoreResult<Option<EndpointId>> {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM endpoints WHERE project_id=$1 AND method=$2 AND path_template=$3 \
             AND merged_into IS NULL",
        )
        .bind(uuid(project))
        .bind(method)
        .bind(path_template)
        .fetch_optional(&mut *self.tx)
        .await
        .map(|found| found.map(id))
        .map_err(unavailable)
    }

    async fn create_endpoint(&mut self, endpoint: &EndpointRow) -> StoreResult<()> {
        sqlx::query("INSERT INTO endpoints(id,project_id,method,path_template) VALUES($1,$2,$3,$4)")
            .bind(uuid(endpoint.id))
            .bind(uuid(endpoint.project_id))
            .bind(&endpoint.method)
            .bind(&endpoint.path_template)
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }

    async fn endpoints(&mut self, project: ProjectId) -> StoreResult<Vec<EndpointRow>> {
        sqlx::query(concat!(
            "SELECT ",
            endpoint_columns!(),
            " FROM endpoints e WHERE e.project_id=$1 AND e.merged_into IS NULL \
             ORDER BY e.created_at,e.id"
        ))
        .bind(uuid(project))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?
        .iter()
        .map(endpoint_row)
        .collect()
    }

    async fn declared_endpoints(&mut self, project: ProjectId) -> StoreResult<Vec<EndpointRow>> {
        sqlx::query(concat!("SELECT ", endpoint_columns!(), " FROM endpoints e WHERE e.project_id=$1 AND e.merged_into IS NULL \
             AND EXISTS(SELECT 1 FROM declarations d WHERE d.endpoint_id=e.id) ORDER BY e.created_at,e.id"))
        .bind(uuid(project))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?
        .iter()
        .map(endpoint_row)
        .collect()
    }

    async fn endpoint(&mut self, endpoint: EndpointId) -> StoreResult<Option<EndpointRow>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            endpoint_columns!(),
            " FROM endpoints a JOIN endpoints e ON e.id=coalesce(a.merged_into,a.id) \
             WHERE a.id=$1"
        ))
        .bind(uuid(endpoint))
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        row.as_ref().map(endpoint_row).transpose()
    }

    async fn aliases(&mut self, endpoint: EndpointId) -> StoreResult<Vec<EndpointId>> {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM endpoints WHERE merged_into=$1 ORDER BY created_at,id",
        )
        .bind(uuid(endpoint))
        .fetch_all(&mut *self.tx)
        .await
        .map(|ids| ids.into_iter().map(id).collect())
        .map_err(unavailable)
    }

    async fn alias(&mut self, from: EndpointId, into: EndpointId) -> StoreResult<()> {
        sqlx::query("UPDATE endpoints SET merged_into=$2 WHERE id=$1 OR merged_into=$1")
            .bind(uuid(from))
            .bind(uuid(into))
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }

    async fn count_fingerprint(
        &mut self,
        traffic: &Traffic,
        hash: &FingerprintHash,
        at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        sqlx::query(
            "UPDATE fingerprints SET calls=calls+1,first_seen=least(first_seen,$5),last_seen=greatest(last_seen,$5) \
             WHERE endpoint_id=$1 AND environment_id=$2 AND address=$3 AND hash=$4",
        )
        .bind(uuid(traffic.endpoint))
        .bind(uuid(traffic.environment_id))
        .bind(traffic.address.as_str())
        .bind(hash.as_slice())
        .bind(at)
        .execute(&mut *self.tx)
        .await
        .map(|done| done.rows_affected() == 1)
        .map_err(unavailable)
    }

    async fn insert_fingerprint(&mut self, fingerprint: &NewFingerprint) -> StoreResult<()> {
        let traffic = &fingerprint.traffic;
        sqlx::query(
            "INSERT INTO fingerprints(endpoint_id,environment_id,address,hash,structure,sample,calls,first_seen,last_seen) \
             VALUES($1,$2,$3,$4,$5,$6,1,$7,$7)",
        )
        .bind(uuid(traffic.endpoint))
        .bind(uuid(traffic.environment_id))
        .bind(traffic.address.as_str())
        .bind(fingerprint.hash.as_slice())
        .bind(&fingerprint.structure)
        .bind(&fingerprint.sample)
        .bind(fingerprint.seen)
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    async fn fingerprints(&mut self, endpoint: EndpointId) -> StoreResult<Vec<FingerprintStats>> {
        sqlx::query(
            "SELECT environment_id,address,structure,calls,first_seen,last_seen FROM fingerprints \
             WHERE endpoint_id=$1 ORDER BY first_seen,hash",
        )
        .bind(uuid(endpoint))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?
        .iter()
        .map(|row| {
            Ok(FingerprintStats {
                environment_id: id(row.try_get("environment_id").map_err(unavailable)?),
                address: address(row.try_get("address").map_err(unavailable)?)?,
                structure: row.try_get("structure").map_err(unavailable)?,
                calls: count(row.try_get("calls").map_err(unavailable)?),
                first_seen: row.try_get("first_seen").map_err(unavailable)?,
                last_seen: row.try_get("last_seen").map_err(unavailable)?,
            })
        })
        .collect()
    }

    async fn traffic(&mut self, project: ProjectId) -> StoreResult<Vec<Traffic>> {
        sqlx::query(
            "SELECT DISTINCT f.endpoint_id,f.environment_id,f.address FROM fingerprints f \
             JOIN endpoints e ON e.id=f.endpoint_id WHERE e.project_id=$1 AND e.merged_into IS NULL \
             ORDER BY 1,2,3",
        )
        .bind(uuid(project))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?
        .iter()
        .map(|row| {
            Ok(Traffic {
                endpoint: id(row.try_get("endpoint_id").map_err(unavailable)?),
                environment_id: id(row.try_get("environment_id").map_err(unavailable)?),
                address: address(row.try_get("address").map_err(unavailable)?)?,
            })
        })
        .collect()
    }

    async fn usage(
        &mut self,
        project: ProjectId,
    ) -> StoreResult<Vec<(EndpointId, EnvironmentUsage)>> {
        sqlx::query(
            "SELECT f.endpoint_id,f.environment_id,sum(f.calls)::bigint AS calls,\
             min(f.first_seen) AS first_seen,max(f.last_seen) AS last_seen FROM fingerprints f \
             JOIN endpoints e ON e.id=f.endpoint_id WHERE e.project_id=$1 AND e.merged_into IS NULL \
             GROUP BY 1,2",
        )
        .bind(uuid(project))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?
        .iter()
        .map(|row| {
            Ok((
                id(row.try_get("endpoint_id").map_err(unavailable)?),
                EnvironmentUsage {
                    environment_id: id(row.try_get("environment_id").map_err(unavailable)?),
                    calls: count(row.try_get("calls").map_err(unavailable)?),
                    first_seen: row.try_get("first_seen").map_err(unavailable)?,
                    last_seen: row.try_get("last_seen").map_err(unavailable)?,
                },
            ))
        })
        .collect()
    }

    async fn move_traffic(&mut self, from: &Traffic, to: EndpointId) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO fingerprints(endpoint_id,environment_id,address,hash,structure,sample,calls,first_seen,last_seen) \
             SELECT $4,environment_id,address,hash,structure,sample,calls,first_seen,last_seen FROM fingerprints \
             WHERE endpoint_id=$1 AND environment_id=$2 AND address=$3 \
             ON CONFLICT(endpoint_id,environment_id,address,hash) DO UPDATE SET \
             calls=fingerprints.calls+excluded.calls, \
             first_seen=least(fingerprints.first_seen,excluded.first_seen), \
             last_seen=greatest(fingerprints.last_seen,excluded.last_seen)",
        )
        .bind(uuid(from.endpoint))
        .bind(uuid(from.environment_id))
        .bind(from.address.as_str())
        .bind(uuid(to))
        .execute(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        sqlx::query(
            "DELETE FROM fingerprints WHERE endpoint_id=$1 AND environment_id=$2 AND address=$3",
        )
        .bind(uuid(from.endpoint))
        .bind(uuid(from.environment_id))
        .bind(from.address.as_str())
        .execute(&mut *self.tx)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    async fn put_declaration(
        &mut self,
        endpoint: EndpointId,
        declaration: &Declaration,
    ) -> StoreResult<bool> {
        sqlx::query(
            "INSERT INTO declarations(endpoint_id,environment_id,platform,source_url,structure,updated_at) \
             VALUES($1,$2,$3,$4,$5,$6) \
             ON CONFLICT(endpoint_id,environment_id,platform,source_url) DO UPDATE SET \
             structure=excluded.structure,updated_at=excluded.updated_at \
             WHERE declarations.updated_at<=excluded.updated_at \
             AND declarations.structure IS DISTINCT FROM excluded.structure",
        )
        .bind(uuid(endpoint))
        .bind(declaration.environment_id.map(uuid))
        .bind(&declaration.platform)
        .bind(&declaration.source_url)
        .bind(&declaration.structure)
        .bind(declaration.updated_at)
        .execute(&mut *self.tx)
        .await
        .map(|done| done.rows_affected() == 1)
        .map_err(unavailable)
    }

    async fn declarations(&mut self, endpoint: EndpointId) -> StoreResult<Vec<Declaration>> {
        sqlx::query(
            "SELECT environment_id,platform,source_url,structure,updated_at FROM declarations \
             WHERE endpoint_id=$1 ORDER BY updated_at",
        )
        .bind(uuid(endpoint))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?
        .iter()
        .map(|row| {
            let environment: Option<Uuid> = row.try_get("environment_id").map_err(unavailable)?;
            Ok(Declaration {
                environment_id: environment.map(id),
                platform: row.try_get("platform").map_err(unavailable)?,
                source_url: row.try_get("source_url").map_err(unavailable)?,
                structure: row.try_get("structure").map_err(unavailable)?,
                updated_at: row.try_get("updated_at").map_err(unavailable)?,
            })
        })
        .collect()
    }

    async fn publish(&mut self, events: &[EndpointEvent]) -> StoreResult<()> {
        // Held until commit, so sequence numbers commit in order and a reader
        // never skips one that commits late.
        sqlx::query("SELECT pg_advisory_xact_lock($1,0)")
            .bind(OUTBOX_LOCK)
            .execute(&mut *self.tx)
            .await
            .map_err(unavailable)?;
        for event in events {
            sqlx::query("INSERT INTO outbox(event) VALUES($1)")
                .bind(serde_json::to_value(event).map_err(unavailable)?)
                .execute(&mut *self.tx)
                .await
                .map_err(unavailable)?;
        }
        Ok(())
    }
}
