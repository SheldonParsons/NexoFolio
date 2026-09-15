//! Current-head lookup and comparison shared by all ingestion protocol versions.
use nexofolio_contracts::{Error, Result};
use nexofolio_intake::PreparedRecord;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Row, Transaction};
use std::collections::HashMap;
use uuid::Uuid;
pub(crate) struct Head {
    pub id: Uuid,
    hash: Option<Vec<u8>>,
    projection: Option<Value>,
}
impl Head {
    pub(crate) fn from_record(id: Uuid, record: &PreparedRecord) -> Self {
        Self {
            id,
            hash: record.structural_hash.clone(),
            projection: record.structural_projection.clone(),
        }
    }
    pub(crate) fn covers(&self, record: &PreparedRecord) -> bool {
        record.structural_hash.is_some()
            && (self.hash == record.structural_hash
                || self
                    .projection
                    .as_ref()
                    .zip(record.structural_projection.as_ref())
                    .is_some_and(|(known, incoming)| {
                        nexofolio_intake::http_projection_covers(known, incoming)
                    }))
    }
}
fn db_error(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "ingestion",
    }
}
pub(crate) async fn load(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    project: Uuid,
    environment_id: Uuid,
    identities: &[String],
) -> Result<HashMap<String, Head>> {
    let previous=sqlx::query(r#"SELECT h.identity_key,h.structural_hash,h.algorithm_version,h.ingestion_id,h.structural_projection,CASE WHEN h.algorithm_version <> 'http-structure-2' THEN i.raw_record ELSE NULL END AS legacy_raw
            FROM ingestion_heads h
            JOIN ingestion_inbox i ON i.id=h.ingestion_id
            WHERE h.project_id=$1
            AND h.environment_id=$2
            AND h.identity_key=ANY($3)"#).bind(project).bind(environment_id).bind(identities).fetch_all(&mut **tx).await.map_err(db_error)?;
    let mut heads = std::collections::HashMap::<String, Head>::new();
    for r in previous {
        let identity: String = r.get("identity_key");
        let mut head = Head {
            hash: r.get("structural_hash"),
            projection: r.get("structural_projection"),
            id: r.get("ingestion_id"),
        };
        if r.get::<String, _>("algorithm_version") != "http-structure-2" {
            // Upgrade only the current head under its existing lock; never scan history.
            head.projection = r
                .get::<Option<Value>, _>("legacy_raw")
                .as_ref()
                .and_then(nexofolio_intake::http_projection);
            head.hash = head
                .projection
                .as_ref()
                .map(|p| Sha256::digest(serde_json::to_vec(p).expect("JSON serializes")).to_vec());
            sqlx::query("UPDATE ingestion_heads SET algorithm_version='http-structure-2',structural_projection=$4,structural_hash=$5 WHERE project_id=$1 AND environment_id=$2 AND identity_key=$3")
                    .bind(project).bind(environment_id).bind(&identity).bind(&head.projection).bind(&head.hash).execute(&mut **tx).await.map_err(db_error)?;
        }
        heads.insert(identity, head);
    }
    Ok(heads)
}

pub(crate) async fn replace(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    project: Uuid,
    env: Uuid,
    id: Uuid,
    record: &PreparedRecord,
) -> Result<Head> {
    sqlx::query("INSERT INTO ingestion_heads(project_id,environment_id,identity_key,algorithm_version,structural_hash,ingestion_id,structural_projection)
        VALUES($1,$2,$3,'http-structure-2',$4,$5,$6)
        ON CONFLICT(project_id,environment_id,identity_key) DO UPDATE SET
        structural_hash=excluded.structural_hash,ingestion_id=excluded.ingestion_id,
        algorithm_version=excluded.algorithm_version,structural_projection=excluded.structural_projection")
        .bind(project).bind(env).bind(&record.identity_key).bind(&record.structural_hash).bind(id).bind(&record.structural_projection)
        .execute(&mut **tx).await.map_err(db_error)?;
    Ok(Head::from_record(id, record))
}
