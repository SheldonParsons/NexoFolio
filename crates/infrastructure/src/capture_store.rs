mod admission;
use crate::Postgres;
use async_trait::async_trait;
use nexofolio_contracts::*;
use nexofolio_evidence::BlobStore;
use nexofolio_intake::{CaptureAdmissionStore, PreparedCapture};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;
#[derive(Clone)]
pub struct PostgresCaptureStore {
    pub(crate) database: Postgres,
    pub(crate) blobs: Arc<dyn BlobStore>,
}
impl PostgresCaptureStore {
    pub fn new(database: Postgres, blobs: Arc<dyn BlobStore>) -> Self {
        Self { database, blobs }
    }
}
pub(crate) fn uuid(v: impl ToString) -> Uuid {
    v.to_string().parse().expect("typed UUID")
}
pub(crate) fn err(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "capture_store",
    }
}
pub(crate) use crate::project_access::authorize_project as authorize;

pub(crate) fn string_kind(k: &CaptureKind) -> &'static str {
    match k {
        CaptureKind::HttpExchange => "http_exchange",
        CaptureKind::PageContext => "page_context",
        CaptureKind::Interaction => "interaction",
        CaptureKind::UiSnapshot => "ui_snapshot",
        CaptureKind::ImageReference => "image_reference",
    }
}

impl PostgresCaptureStore {
    pub async fn put_asset(
        &self,
        user: UserId,
        project: ProjectId,
        id: Uuid,
        media: &str,
        bytes: Vec<u8>,
    ) -> Result<AssetReceipt> {
        let valid = match media {
            "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "image/jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
            "image/webp" => {
                bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
            }
            _ => false,
        };
        if !valid || bytes.len() > 8 * 1024 * 1024 {
            return Err(Error::invalid("unsupported or oversized image"));
        }
        let mut tx = self.database.pool.begin().await.map_err(err)?;
        authorize(&mut tx, user, project).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("asset:{project}:{id}"))
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        let content = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("blob:{project}:{content}"))
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        let blob = self.blobs.put(project, bytes, media).await?;
        let previous: Option<String> =
            sqlx::query_scalar("SELECT sha256 FROM capture_assets WHERE project_id=$1 AND id=$2")
                .bind(uuid(project))
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(err)?;
        if previous.as_ref().is_some_and(|p| p != &blob.sha256) {
            return Err(Error::Conflict);
        }
        sqlx::query("INSERT INTO capture_blobs(project_id,sha256,bytes,media_type) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(uuid(project)).bind(&blob.sha256).bind(blob.bytes as i64).bind(media).execute(&mut *tx).await.map_err(err)?;
        sqlx::query("INSERT INTO capture_assets(project_id,id,sha256,actor_id) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(uuid(project)).bind(id).bind(&blob.sha256).bind(uuid(user)).execute(&mut *tx).await.map_err(err)?;
        tx.commit().await.map_err(err)?;
        Ok(AssetReceipt {
            asset_id: id,
            project_id: project,
            sha256: blob.sha256,
            bytes: blob.bytes,
            media_type: media.into(),
            replayed: previous.is_some(),
        })
    }
    pub async fn asset(
        &self,
        user: UserId,
        project: ProjectId,
        id: Uuid,
    ) -> Result<(String, Vec<u8>)> {
        let mut tx = self.database.pool.begin().await.map_err(err)?;
        authorize(&mut tx, user, project).await?;
        let r=sqlx::query("SELECT b.sha256,b.media_type FROM capture_assets a JOIN capture_blobs b ON b.project_id=a.project_id AND b.sha256=a.sha256 WHERE a.project_id=$1 AND a.id=$2").bind(uuid(project)).bind(id).fetch_optional(&mut *tx).await.map_err(err)?.ok_or(Error::NotFound)?;
        let bytes = self
            .blobs
            .get(project, &r.get::<String, _>("sha256"))
            .await?;
        tx.commit().await.map_err(err)?;
        Ok((r.get("media_type"), bytes))
    }
    pub async fn observation(
        &self,
        user: UserId,
        project: ProjectId,
        id: Uuid,
    ) -> Result<CaptureObservation> {
        let mut tx = self.database.pool.begin().await.map_err(err)?;
        authorize(&mut tx, user, project).await?;
        let row=sqlx::query("SELECT id,environment_id,kind,record_id,actor_id,captured_at,context,structure,ingestion_id,evidence_status,error_code,raw_hash FROM capture_events WHERE project_id=$1 AND id=$2").bind(uuid(project)).bind(id).fetch_optional(&mut *tx).await.map_err(err)?.ok_or(Error::NotFound)?;
        let hash: Option<String> = row.get("raw_hash");
        let payload = match &hash {
            Some(h) => Some(
                serde_json::from_slice::<Value>(&self.blobs.get(project, h).await?).map_err(
                    |_| Error::Unavailable {
                        component: "capture_payload",
                    },
                )?,
            ),
            None => None,
        };
        let kind: CaptureKind = serde_json::from_value(json!(row.get::<String, _>("kind")))
            .map_err(|_| Error::Unavailable {
                component: "capture_kind",
            })?;
        let result = CaptureObservation {
            id,
            project_id: project,
            environment_id: row
                .get::<Uuid, _>("environment_id")
                .to_string()
                .parse()
                .unwrap(),
            kind,
            record_id: row.get("record_id"),
            actor_id: row.get::<Uuid, _>("actor_id").to_string().parse().unwrap(),
            captured_at: row
                .get::<chrono::DateTime<chrono::Utc>, _>("captured_at")
                .to_rfc3339(),
            context: row
                .get::<Option<Value>, _>("context")
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Unavailable {
                    component: "capture_context",
                })?,
            structure: row.get("structure"),
            ingestion_id: row.get("ingestion_id"),
            evidence_status: row.get("evidence_status"),
            error_code: row.get("error_code"),
            payload_available: payload.is_some(),
            payload,
        };
        tx.commit().await.map_err(err)?;
        Ok(result)
    }
    pub async fn facts(
        &self,
        user: UserId,
        project: ProjectId,
        page: u32,
        limit: u32,
        environment: Option<EnvironmentId>,
    ) -> Result<EvidencePage> {
        if page == 0 || page > 100000 || limit == 0 || limit > 100 {
            return Err(Error::invalid("invalid evidence pagination"));
        }
        let mut tx = self.database.pool.begin().await.map_err(err)?;
        authorize(&mut tx, user, project).await?;
        let total:i64=sqlx::query_scalar("SELECT count(*) FROM evidence_facts WHERE project_id=$1 AND data->>'superseded' IS DISTINCT FROM 'true' AND ($2::uuid IS NULL OR environment_id=$2)").bind(uuid(project)).bind(environment.map(uuid)).fetch_one(&mut *tx).await.map_err(err)?;
        let rows=sqlx::query(r#"SELECT jsonb_build_object('id',f.id,'project_id',f.project_id,'environment_id',f.environment_id,'kind',f.kind,'subject',f.subject,'data',f.data,'observations',f.observations,'first_seen',f.first_seen,'last_seen',f.last_seen,'samples',(SELECT coalesce(jsonb_agg(event_id
            ORDER BY event_id),'[]'::jsonb)
            FROM evidence_samples s
            WHERE s.fact_id=f.id)) AS value
            FROM evidence_facts f
            WHERE f.project_id=$1
            AND f.data->>'superseded' IS DISTINCT FROM 'true'
            AND ($2::uuid IS NULL
            OR environment_id=$2)
            ORDER BY f.last_seen DESC,f.id
            LIMIT $3 OFFSET $4"#).bind(uuid(project)).bind(environment.map(uuid)).bind(i64::from(limit)).bind(i64::from(page-1)*i64::from(limit)).fetch_all(&mut *tx).await.map_err(err)?;
        let items = rows
            .into_iter()
            .map(|r| {
                serde_json::from_value(r.get("value")).map_err(|_| Error::Unavailable {
                    component: "evidence_contract",
                })
            })
            .collect::<Result<_>>()?;
        tx.commit().await.map_err(err)?;
        Ok(EvidencePage {
            items,
            page,
            limit,
            total,
        })
    }
}

impl PostgresCaptureStore {
    pub async fn fact(&self, user: UserId, project: ProjectId, fact: Uuid) -> Result<EvidenceFact> {
        let mut tx = self.database.pool.begin().await.map_err(err)?;
        authorize(&mut tx, user, project).await?;
        let value:Value=sqlx::query_scalar(r#"SELECT to_jsonb(f)-'key_hash'-'subject_hash' || jsonb_build_object('samples',coalesce((SELECT jsonb_agg(event_id
            ORDER BY event_id)
            FROM evidence_samples s
            WHERE s.fact_id=f.id),'[]'::jsonb))
            FROM evidence_facts f
            WHERE project_id=$1
            AND id=$2"#).bind(uuid(project)).bind(fact).fetch_optional(&mut *tx).await.map_err(err)?.ok_or(Error::NotFound)?;
        let out = serde_json::from_value(value).map_err(|_| Error::Unavailable {
            component: "evidence_contract",
        })?;
        tx.commit().await.map_err(err)?;
        Ok(out)
    }
}

#[async_trait]
impl nexofolio_evidence::CaptureEvidenceRepository for PostgresCaptureStore {
    async fn put_asset(
        &self,
        user: UserId,
        project: ProjectId,
        id: Uuid,
        media: &str,
        bytes: Vec<u8>,
    ) -> Result<AssetReceipt> {
        Self::put_asset(self, user, project, id, media, bytes).await
    }
    async fn asset(&self, user: UserId, project: ProjectId, id: Uuid) -> Result<(String, Vec<u8>)> {
        Self::asset(self, user, project, id).await
    }
    async fn observation(
        &self,
        user: UserId,
        project: ProjectId,
        id: Uuid,
    ) -> Result<CaptureObservation> {
        Self::observation(self, user, project, id).await
    }
    async fn fact(&self, user: UserId, project: ProjectId, id: Uuid) -> Result<EvidenceFact> {
        Self::fact(self, user, project, id).await
    }
    async fn facts(
        &self,
        user: UserId,
        project: ProjectId,
        page: u32,
        limit: u32,
        environment: Option<EnvironmentId>,
    ) -> Result<EvidencePage> {
        Self::facts(self, user, project, page, limit, environment).await
    }
}
