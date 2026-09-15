use crate::Postgres;
use async_trait::async_trait;
use nexofolio_contracts::{EnvironmentId, Error, InterfaceId, ProjectId, Result, UserId};
use nexofolio_knowledge::{
    ClaimedObservation, DocumentQuery, DocumentReader, ObservationProcessor, ObservedDefinition,
    ProcessingResult,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone)]
pub struct PostgresDocuments {
    database: Postgres,
}
impl PostgresDocuments {
    pub fn new(database: Postgres) -> Self {
        Self { database }
    }
}
fn db_error(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "documents",
    }
}
fn uuid(v: impl ToString) -> Uuid {
    v.to_string().parse().expect("typed UUID")
}
fn typed<T: std::str::FromStr>(v: Uuid) -> T
where
    T::Err: std::fmt::Debug,
{
    v.to_string().parse().expect("database UUID")
}
#[async_trait]
impl ObservationProcessor for PostgresDocuments {
    async fn claim(&self) -> Result<Option<ClaimedObservation>> {
        // Exhausted abandoned executions remain visible and block later observations of that key.
        sqlx::query(
            r#"UPDATE ingestion_inbox
            SET status='failed',last_error_code='LEASE_EXHAUSTED',lease_until=NULL
            WHERE status='processing'
            AND lease_until<clock_timestamp()
            AND processing_attempts>=5"#,
        )
        .execute(&self.database.pool)
        .await
        .map_err(db_error)?;
        let row=sqlx::query(r#"WITH candidate AS (SELECT i.id
            FROM ingestion_inbox i
            WHERE ((i.status='pending'
            AND i.retry_at<=clock_timestamp())
            OR (i.status='processing'
            AND i.lease_until<clock_timestamp()))
            AND i.processing_attempts<5
            AND NOT EXISTS(SELECT 1
            FROM ingestion_inbox earlier
            WHERE earlier.project_id=i.project_id
            AND earlier.environment_id=i.environment_id
            AND earlier.identity_key=i.identity_key
            AND earlier.status<>'completed'
            AND (earlier.received_at,earlier.id)<(i.received_at,i.id))
            ORDER BY i.received_at,i.id FOR UPDATE OF i SKIP LOCKED
            LIMIT 1) UPDATE ingestion_inbox i
            SET status='processing',processing_generation=processing_generation+1,processing_attempts=processing_attempts+1,lease_until=clock_timestamp()+interval '120 seconds',last_error_code=NULL
            FROM candidate c
            WHERE i.id=c.id
            RETURNING i.id,i.project_id,i.environment_id,i.identity_key,i.processing_generation,i.raw_record,i.path_identity"#)
   .fetch_optional(&self.database.pool).await.map_err(db_error)?;
        Ok(row.map(|r| ClaimedObservation {
            ingestion_id: r.get("id"),
            project_id: typed(r.get("project_id")),
            environment_id: typed(r.get("environment_id")),
            identity_key: r.get("identity_key"),
            path_identity: r
                .get::<Option<Value>, _>("path_identity")
                .map(|v| serde_json::from_value(v).expect("stored identity")),
            generation: r.get("processing_generation"),
            raw: r.get("raw_record"),
        }))
    }
    async fn finish(
        &self,
        claim: &ClaimedObservation,
        mut definition: ObservedDefinition,
    ) -> Result<ProcessingResult> {
        if let Some(path) = &claim.path_identity {
            nexofolio_knowledge::apply_observed_path(&mut definition, path)?;
        }
        if format!("{} {}", definition.method, definition.path) != claim.identity_key {
            return Err(Error::InvalidInput {
                message: "IDENTITY_MISMATCH".into(),
            });
        }
        let value = serde_json::to_value(&definition).map_err(|_| Error::InvalidInput {
            message: "INVALID_DEFINITION".into(),
        })?;
        let hash = Sha256::digest(serde_json::to_vec(&value).map_err(|_| Error::InvalidInput {
            message: "INVALID_DEFINITION".into(),
        })?)
        .to_vec();
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        let valid: Option<Uuid> = sqlx::query_scalar(
            r#"SELECT id
            FROM ingestion_inbox
            WHERE id=$1
            AND project_id=$2
            AND environment_id=$3
            AND identity_key=$4
            AND status='processing'
            AND processing_generation=$5
            AND lease_until>clock_timestamp() FOR UPDATE"#,
        )
        .bind(claim.ingestion_id)
        .bind(uuid(claim.project_id))
        .bind(uuid(claim.environment_id))
        .bind(&claim.identity_key)
        .bind(claim.generation)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?;
        if valid.is_none() {
            return Err(Error::Conflict);
        }
        let interface: Uuid = sqlx::query_scalar(
            r#"INSERT INTO interface_documents(id,project_id,identity_key,method,path)
            VALUES($1,$2,$3,$4,$5)
            ON CONFLICT(project_id,identity_key) DO UPDATE
            SET identity_key=excluded.identity_key
            RETURNING id"#,
        )
        .bind(Uuid::new_v4())
        .bind(uuid(claim.project_id))
        .bind(&claim.identity_key)
        .bind(&definition.method)
        .bind(&definition.path)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
        let current = sqlx::query(
            r#"SELECT r.id,r.definition_hash,r.definition
            FROM interface_environment_current c
            JOIN interface_observed_revisions r ON r.id=c.current_revision_id
            WHERE c.interface_id=$1
            AND c.environment_id=$2"#,
        )
        .bind(interface)
        .bind(uuid(claim.environment_id))
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?;
        let (revision, difference, outcome) = if let Some(row) = current {
            let revision: Uuid = row.get("id");
            if row.get::<Vec<u8>, _>("definition_hash") == hash
                || nexofolio_knowledge::definition_covers(
                    &row.get::<Value, _>("definition"),
                    &value,
                )
            {
                (revision, None, "unchanged")
            } else {
                let id:Uuid=sqlx::query_scalar(r#"INSERT INTO interface_observed_differences(id,interface_id,environment_id,base_revision_id,proposed_definition,definition_hash,origin_ingestion_id)
            VALUES($1,$2,$3,$4,$5,$6,$7)
            ON CONFLICT(base_revision_id,definition_hash) DO UPDATE
            SET definition_hash=excluded.definition_hash
            RETURNING id"#)
     .bind(Uuid::new_v4()).bind(interface).bind(uuid(claim.environment_id)).bind(revision).bind(&value).bind(&hash).bind(claim.ingestion_id).fetch_one(&mut *tx).await.map_err(db_error)?;
                (revision, Some(id), "difference_recorded")
            }
        } else {
            let revision = Uuid::new_v4();
            sqlx::query("INSERT INTO interface_observed_revisions(id,project_id,interface_id,environment_id,definition,definition_hash,origin_ingestion_id) VALUES($1,$2,$3,$4,$5,$6,$7)")
    .bind(revision).bind(uuid(claim.project_id)).bind(interface).bind(uuid(claim.environment_id)).bind(&value).bind(&hash).bind(claim.ingestion_id).execute(&mut *tx).await.map_err(db_error)?;
            sqlx::query("INSERT INTO interface_environment_current(interface_id,environment_id,current_revision_id) VALUES($1,$2,$3)").bind(interface).bind(uuid(claim.environment_id)).bind(revision).execute(&mut *tx).await.map_err(db_error)?;
            (revision, None, "created")
        };
        sqlx::query("INSERT INTO interface_observations(ingestion_id,interface_id,environment_id,compared_revision_id,difference_id,outcome) VALUES($1,$2,$3,$4,$5,$6)")
   .bind(claim.ingestion_id).bind(interface).bind(uuid(claim.environment_id)).bind(revision).bind(difference).bind(outcome).execute(&mut *tx).await.map_err(db_error)?;
        let updated=sqlx::query(r#"UPDATE ingestion_inbox
            SET status='completed',lease_until=NULL,completed_at=clock_timestamp(),last_error_code=NULL
            WHERE id=$1
            AND processing_generation=$2
            AND lease_until>clock_timestamp()"#)
   .bind(claim.ingestion_id).bind(claim.generation).execute(&mut *tx).await.map_err(db_error)?.rows_affected();
        if updated != 1 {
            return Err(Error::Conflict);
        }
        tx.commit().await.map_err(db_error)?;
        Ok(ProcessingResult {
            ingestion_id: claim.ingestion_id,
            interface_id: typed(interface),
            outcome: outcome.into(),
        })
    }
    async fn fail(&self, c: &ClaimedObservation, code: &str) -> Result<()> {
        let code = match code {
            "INVALID_OBSERVATION" => "INVALID_OBSERVATION",
            _ => "DOCUMENT_COMMIT_FAILED",
        };
        let n=sqlx::query(r#"UPDATE ingestion_inbox
            SET status=CASE WHEN processing_attempts>=5 THEN 'failed' ELSE 'pending' END,lease_until=NULL,retry_at=clock_timestamp()+make_interval(secs=>power(2,least(processing_attempts,5))::double precision),last_error_code=$3
            WHERE id=$1
            AND processing_generation=$2
            AND status='processing'
            AND lease_until>clock_timestamp()"#)
   .bind(c.ingestion_id).bind(c.generation).bind(code).execute(&self.database.pool).await.map_err(db_error)?.rows_affected();
        if n != 1 {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    async fn retry_failed(&self, id: Uuid) -> Result<()> {
        let n=sqlx::query(r#"UPDATE ingestion_inbox
            SET status='pending',processing_attempts=0,processing_generation=processing_generation+1,retry_at=clock_timestamp(),lease_until=NULL,last_error_code=NULL
            WHERE id=$1
            AND status='failed'"#).bind(id).execute(&self.database.pool).await.map_err(db_error)?.rows_affected();
        if n != 1 {
            return Err(Error::Conflict);
        }
        Ok(())
    }
}

impl PostgresDocuments {
    async fn authorize(&self, user: UserId, project: ProjectId) -> Result<()> {
        let row = sqlx::query(
            r#"SELECT u.grants_synced,EXISTS(SELECT 1
            FROM user_project_access a
            WHERE a.user_id=u.id
            AND a.project_id=$2) AS allowed
            FROM users u
            JOIN projects p ON p.instance=u.instance
            WHERE u.id=$1
            AND p.id=$2
            AND u.enabled"#,
        )
        .bind(uuid(user))
        .bind(uuid(project))
        .fetch_optional(&self.database.pool)
        .await
        .map_err(db_error)?
        .ok_or(Error::NotFound)?;
        if !row.get::<bool, _>("grants_synced") {
            return Err(Error::Unavailable {
                component: "project_access",
            });
        }
        if !row.get::<bool, _>("allowed") {
            return Err(Error::Forbidden);
        }
        Ok(())
    }
    async fn environment(&self, project: ProjectId, environment: EnvironmentId) -> Result<()> {
        let yes: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM environments WHERE project_id=$1 AND id=$2)",
        )
        .bind(uuid(project))
        .bind(uuid(environment))
        .fetch_one(&self.database.pool)
        .await
        .map_err(db_error)?;
        if !yes {
            return Err(Error::NotFound);
        }
        Ok(())
    }
}
fn pagination(page: u32, limit: u32) -> Result<()> {
    if page == 0 || page > 100000 || !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput {
            message: "invalid pagination".into(),
        });
    }
    Ok(())
}
#[async_trait]
impl DocumentReader for PostgresDocuments {
    async fn list(&self, user: UserId, project: ProjectId, q: DocumentQuery) -> Result<Value> {
        pagination(q.page, q.limit)?;
        self.authorize(user, project).await?;
        self.environment(project, q.environment_id).await?;
        let query = q.query.unwrap_or_default();
        if query.len() > 500 {
            return Err(Error::InvalidInput {
                message: "query too long".into(),
            });
        }
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let total: i64 = sqlx::query_scalar(
            r#"SELECT count(*)
            FROM interface_documents d
            JOIN interface_environment_current c ON c.interface_id=d.id
            WHERE d.project_id=$1
            AND c.environment_id=$2
            AND (strpos(lower(d.path),lower($3))>0
            OR strpos(lower(d.method),lower($3))>0)"#,
        )
        .bind(uuid(project))
        .bind(uuid(q.environment_id))
        .bind(&query)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
        let rows = sqlx::query(
            r#"SELECT d.id,d.method,d.path,c.current_revision_id,(SELECT count(*)
            FROM interface_observed_differences f
            WHERE f.base_revision_id=c.current_revision_id) AS differences,EXISTS(SELECT 1
            FROM project_catalogs pc
            JOIN catalog_version_assignments ca ON ca.version_id=pc.current_version_id
            WHERE pc.project_id=d.project_id
            AND ca.interface_id=d.id
            AND ca.directory_id IS NOT NULL) AS classified
            FROM interface_documents d
            JOIN interface_environment_current c ON c.interface_id=d.id
            WHERE d.project_id=$1
            AND c.environment_id=$2
            AND (strpos(lower(d.path),lower($3))>0
            OR strpos(lower(d.method),lower($3))>0)
            ORDER BY d.method,d.path,d.id
            LIMIT $4 OFFSET $5"#,
        )
        .bind(uuid(project))
        .bind(uuid(q.environment_id))
        .bind(&query)
        .bind(i64::from(q.limit))
        .bind(i64::from((q.page - 1) * q.limit))
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
        let items:Vec<_>=rows.iter().map(|r|json!({"interface_id":r.get::<Uuid,_>("id"),"method":r.get::<String,_>("method"),"path":r.get::<String,_>("path"),"environment_id":q.environment_id,"revision_id":r.get::<Uuid,_>("current_revision_id"),"state":"observed","classification":if r.get::<bool,_>("classified"){"classified"}else{"unclassified"},"pending_difference_count":r.get::<i64,_>("differences")})).collect();
        tx.commit().await.map_err(db_error)?;
        Ok(
            json!({"items":items,"total":total,"page":q.page,"limit":q.limit,"environment_id":q.environment_id}),
        )
    }
    async fn detail(
        &self,
        user: UserId,
        project: ProjectId,
        env: EnvironmentId,
        id: InterfaceId,
    ) -> Result<Value> {
        self.authorize(user, project).await?;
        let row=sqlx::query(r#"SELECT d.method,d.path,r.id AS revision_id,r.definition,r.origin_ingestion_id,r.created_at,e.name AS environment_name,(SELECT count(*)
            FROM interface_observed_differences f
            WHERE f.base_revision_id=r.id) AS differences,EXISTS(SELECT 1
            FROM project_catalogs pc
            JOIN catalog_version_assignments ca ON ca.version_id=pc.current_version_id
            WHERE pc.project_id=d.project_id
            AND ca.interface_id=d.id
            AND ca.directory_id IS NOT NULL) AS classified
            FROM interface_documents d
            JOIN interface_environment_current c ON c.interface_id=d.id
            JOIN interface_observed_revisions r ON r.id=c.current_revision_id
            JOIN environments e ON e.id=c.environment_id
            WHERE d.project_id=$1
            AND d.id=$2
            AND c.environment_id=$3"#)
   .bind(uuid(project)).bind(uuid(id)).bind(uuid(env)).fetch_optional(&self.database.pool).await.map_err(db_error)?.ok_or(Error::NotFound)?;
        Ok(
            json!({"interface_id":id,"project_id":project,"environment":{"id":env,"name":row.get::<String,_>("environment_name")},"method":row.get::<String,_>("method"),"path":row.get::<String,_>("path"),"revision_id":row.get::<Uuid,_>("revision_id"),"state":"observed","classification":if row.get::<bool,_>("classified"){"classified"}else{"unclassified"},"definition":row.get::<Value,_>("definition"),"origin_ingestion_id":row.get::<Uuid,_>("origin_ingestion_id"),"created_at":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at").to_rfc3339(),"pending_difference_count":row.get::<i64,_>("differences")}),
        )
    }
    async fn observations(
        &self,
        user: UserId,
        project: ProjectId,
        env: EnvironmentId,
        id: InterfaceId,
        page: u32,
        limit: u32,
    ) -> Result<Value> {
        pagination(page, limit)?;
        self.detail(user, project, env, id).await?;
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let total:i64=sqlx::query_scalar("SELECT count(*) FROM interface_observations WHERE interface_id=$1 AND environment_id=$2").bind(uuid(id)).bind(uuid(env)).fetch_one(&mut *tx).await.map_err(db_error)?;
        let rows = sqlx::query(
            r#"SELECT ingestion_id,outcome,difference_id,compared_revision_id,processed_at
            FROM interface_observations
            WHERE interface_id=$1
            AND environment_id=$2
            ORDER BY processed_at,ingestion_id
            LIMIT $3 OFFSET $4"#,
        )
        .bind(uuid(id))
        .bind(uuid(env))
        .bind(i64::from(limit))
        .bind(i64::from((page - 1) * limit))
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
        let items:Vec<_>=rows.iter().map(|r|json!({"ingestion_id":r.get::<Uuid,_>("ingestion_id"),"outcome":r.get::<String,_>("outcome"),"difference_id":r.get::<Option<Uuid>,_>("difference_id"),"compared_revision_id":r.get::<Uuid,_>("compared_revision_id"),"processed_at":r.get::<chrono::DateTime<chrono::Utc>,_>("processed_at").to_rfc3339()})).collect();
        tx.commit().await.map_err(db_error)?;
        Ok(json!({"items":items,"total":total,"page":page,"limit":limit}))
    }
    async fn observation(&self, user: UserId, project: ProjectId, id: Uuid) -> Result<Value> {
        self.authorize(user, project).await?;
        let row=sqlx::query(r#"SELECT i.id,i.environment_id,i.identity_key,i.status,i.raw_record,i.processing_attempts,i.last_error_code,o.interface_id,o.outcome,o.compared_revision_id,o.difference_id,f.proposed_definition
            FROM ingestion_inbox i
            LEFT JOIN interface_observations o ON o.ingestion_id=i.id
            LEFT JOIN interface_observed_differences f ON f.id=o.difference_id
            WHERE i.project_id=$1
            AND i.id=$2"#).bind(uuid(project)).bind(id).fetch_optional(&self.database.pool).await.map_err(db_error)?.ok_or(Error::NotFound)?;
        Ok(
            json!({"ingestion_id":id,"project_id":project,"environment_id":row.get::<Uuid,_>("environment_id"),"status":row.get::<String,_>("status"),"attempts":row.get::<i32,_>("processing_attempts"),"error_code":row.get::<Option<String>,_>("last_error_code"),"interface_id":row.get::<Option<Uuid>,_>("interface_id"),"outcome":row.get::<Option<String>,_>("outcome"),"compared_revision_id":row.get::<Option<Uuid>,_>("compared_revision_id"),"difference_id":row.get::<Option<Uuid>,_>("difference_id"),"proposed_definition":row.get::<Option<Value>,_>("proposed_definition"),"raw_record":row.get::<Option<Value>,_>("raw_record")}),
        )
    }
}
