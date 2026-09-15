use crate::Postgres;
use async_trait::async_trait;
use nexofolio_contracts::*;
use nexofolio_knowledge::CatalogPreviewStore;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone)]
pub struct PostgresCatalogPreviews {
    database: Postgres,
}
impl PostgresCatalogPreviews {
    pub fn new(database: Postgres) -> Self {
        Self { database }
    }
}
fn uuid(id: impl ToString) -> Uuid {
    id.to_string().parse().expect("typed UUID")
}
fn db_error(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "catalog_previews",
    }
}
fn invalid() -> Error {
    Error::InvalidInput {
        message: "catalog snapshot or candidate exceeds preview limits".into(),
    }
}
fn decode(row: sqlx::postgres::PgRow) -> Result<PreviewTask> {
    serde_json::from_value(row.get::<Value, _>("value")).map_err(|_| Error::Unavailable {
        component: "catalog_preview_contract",
    })
}
const READ: &str = "SELECT jsonb_build_object('task_id',id,'candidate_id',candidate_id,'contract_version',contract_version,'status',status,'snapshot_at',snapshot_at,'snapshot_sha256',snapshot_sha256,'snapshot',snapshot,'generation',generation,'generator',generator,'error_code',error_code,'candidate',candidate,'review',review) AS value FROM catalog_preview_tasks WHERE id=$1";
const READ_PROJECT: &str = "SELECT jsonb_build_object('task_id',id,'candidate_id',candidate_id,'contract_version',contract_version,'status',status,'snapshot_at',snapshot_at,'snapshot_sha256',snapshot_sha256,'snapshot',snapshot,'generation',generation,'generator',generator,'error_code',error_code,'candidate',candidate,'review',review) AS value FROM catalog_preview_tasks WHERE id=$1 AND project_id=$2";
#[async_trait]
impl CatalogPreviewStore for PostgresCatalogPreviews {
    async fn create(&self, project: ProjectId) -> Result<PreviewTask> {
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let name: String = sqlx::query_scalar("SELECT name FROM projects WHERE id=$1")
            .bind(uuid(project))
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_error)?
            .ok_or(Error::NotFound)?;
        let size=sqlx::query(r#"SELECT count(DISTINCT d.id) AS interfaces,coalesce(sum(octet_length(r.definition::text)),0)::bigint AS bytes
            FROM interface_documents d
            JOIN interface_environment_current c ON c.interface_id=d.id
            JOIN interface_observed_revisions r ON r.id=c.current_revision_id
            WHERE d.project_id=$1"#)
            .bind(uuid(project)).fetch_one(&mut *tx).await.map_err(db_error)?;
        if size.get::<i64, _>("interfaces") == 0 {
            return Err(Error::InvalidInput {
                message: "project has no observed interfaces".into(),
            });
        }
        if size.get::<i64, _>("interfaces") > MAX_PREVIEW_INTERFACES as i64
            || size.get::<i64, _>("bytes") > MAX_PREVIEW_BYTES as i64
        {
            return Err(invalid());
        }
        let rows=sqlx::query(r#"SELECT d.id,d.method,d.path,jsonb_agg(jsonb_build_object('environment_id',e.id,'environment_name',e.name,'revision_id',r.id,'definition',r.definition)
            ORDER BY e.id) AS environments
            FROM interface_documents d
            JOIN interface_environment_current c ON c.interface_id=d.id
            JOIN interface_observed_revisions r ON r.id=c.current_revision_id
            JOIN environments e ON e.id=c.environment_id
            WHERE d.project_id=$1
            GROUP BY d.id,d.method,d.path
            ORDER BY d.method,d.path,d.id"#)
            .bind(uuid(project)).fetch_all(&mut *tx).await.map_err(db_error)?;
        let values:Vec<_>=rows.iter().map(|r|json!({"interface_id":r.get::<Uuid,_>("id"),"method":r.get::<String,_>("method"),"path":r.get::<String,_>("path"),"environments":r.get::<Value,_>("environments")})).collect();
        let mut snapshot: CatalogSnapshot = serde_json::from_value(
            json!({"project_id":project,"project_name":name,"interfaces":values}),
        )
        .map_err(|_| invalid())?;
        let policy: Value = sqlx::query_scalar("SELECT path_policy FROM projects WHERE id=$1")
            .bind(uuid(project))
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
        let policy: PathPolicy =
            serde_json::from_value(policy).map_err(|_| Error::Unavailable {
                component: "path_policy",
            })?;
        if !policy.valid() {
            return Err(Error::Unavailable {
                component: "path_policy",
            });
        }
        for interface in &mut snapshot.interfaces {
            interface.recognized_path = Some(identify_path(&interface.path, &policy));
        }
        let encoded = serde_json::to_vec(&snapshot).expect("serializes");
        if encoded.len() > MAX_PREVIEW_BYTES {
            return Err(invalid());
        }
        let hash = Sha256::digest(&encoded)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let task = JobId::new();
        let candidate = CatalogVersion::new();
        sqlx::query("INSERT INTO catalog_preview_tasks(id,project_id,candidate_id,contract_version,snapshot_at,snapshot,snapshot_sha256) VALUES($1,$2,$3,$4,transaction_timestamp(),$5,$6)")
            .bind(uuid(task)).bind(uuid(project)).bind(uuid(candidate)).bind(CATALOG_PREVIEW_VERSION).bind(serde_json::to_value(snapshot).expect("serializes")).bind(hash).execute(&mut *tx).await.map_err(db_error)?;
        let result = decode(
            sqlx::query(READ)
                .bind(uuid(task))
                .fetch_one(&mut *tx)
                .await
                .map_err(db_error)?,
        )?;
        tx.commit().await.map_err(db_error)?;
        Ok(result)
    }
    async fn read(&self, task: JobId) -> Result<PreviewTask> {
        decode(
            sqlx::query(READ)
                .bind(uuid(task))
                .fetch_optional(&self.database.pool)
                .await
                .map_err(db_error)?
                .ok_or(Error::NotFound)?,
        )
    }
    async fn claim(&self, task: JobId, generator: &GeneratorInfo) -> Result<PreviewTask> {
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        let row=sqlx::query(r#"UPDATE catalog_preview_tasks
            SET status='running',generation=generation+1,lease_until=clock_timestamp()+interval '5 minutes',generator=$2,error_code=NULL,finished_at=NULL
            WHERE id=$1
            AND contract_version=$3
            AND (status IN ('pending','failed')
            OR (status='running'
            AND lease_until<=clock_timestamp()))
            RETURNING id"#)
            .bind(uuid(task)).bind(serde_json::to_value(generator).expect("serializes")).bind(CATALOG_PREVIEW_VERSION).fetch_optional(&mut *tx).await.map_err(db_error)?;
        if row.is_none() {
            return Err(Error::Conflict);
        }
        let result = decode(
            sqlx::query(READ)
                .bind(uuid(task))
                .fetch_one(&mut *tx)
                .await
                .map_err(db_error)?,
        )?;
        tx.commit().await.map_err(db_error)?;
        Ok(result)
    }
    async fn complete(
        &self,
        task: &PreviewTask,
        candidate: &DirectoryCandidate,
        review: &PreviewReview,
    ) -> Result<()> {
        let encoded = serde_json::to_value(candidate).expect("serializes");
        if serde_json::to_vec(&encoded).expect("serializes").len() > MAX_CANDIDATE_BYTES {
            return Err(invalid());
        }
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        let changed = sqlx::query(
            r#"UPDATE catalog_preview_tasks
            SET status=$3,candidate=$4,review=$5,lease_until=NULL,finished_at=clock_timestamp()
            WHERE id=$1
            AND generation=$2
            AND status='running'
            AND lease_until>clock_timestamp()
            AND candidate_id=$6
            RETURNING snapshot"#,
        )
        .bind(uuid(task.task_id))
        .bind(task.generation)
        .bind(if review.structurally_valid {
            "ready"
        } else {
            "rejected"
        })
        .bind(encoded)
        .bind(serde_json::to_value(review).expect("serializes"))
        .bind(uuid(task.candidate_id))
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?
        .ok_or(Error::Conflict)?;
        let canonical: CatalogSnapshot =
            serde_json::from_value(changed.get("snapshot")).map_err(|_| Error::Conflict)?;
        // Invalid results are retained for inspection, never materialized as a usable directory.
        if review.structurally_valid {
            for node in &candidate.nodes {
                sqlx::query("INSERT INTO catalog_preview_nodes(candidate_id,id,parent_id,name,description) VALUES($1,$2,$3,$4,$5)")
                    .bind(uuid(task.candidate_id)).bind(uuid(node.id)).bind(node.parent.map(uuid)).bind(&node.name).bind(&node.description).execute(&mut *tx).await.map_err(db_error)?;
            }
            for item in &candidate.assignments {
                if !canonical
                    .interfaces
                    .iter()
                    .any(|i| i.interface_id == item.interface_id)
                {
                    return Err(Error::Conflict);
                }
                sqlx::query("INSERT INTO catalog_preview_assignments(candidate_id,interface_id,directory_id,reason) VALUES($1,$2,$3,$4)")
                    .bind(uuid(task.candidate_id)).bind(uuid(item.interface_id)).bind(item.directory_id.map(uuid)).bind(&item.reason).execute(&mut *tx).await.map_err(db_error)?;
            }
        }
        tx.commit().await.map_err(db_error)?;
        Ok(())
    }
    async fn fail(&self, task: &PreviewTask, code: &'static str) -> Result<()> {
        let result = sqlx::query(
            r#"UPDATE catalog_preview_tasks
            SET status='failed',error_code=$3,lease_until=NULL,finished_at=clock_timestamp()
            WHERE id=$1
            AND generation=$2
            AND status='running'
            AND lease_until>clock_timestamp()"#,
        )
        .bind(uuid(task.task_id))
        .bind(task.generation)
        .bind(code)
        .execute(&self.database.pool)
        .await
        .map_err(db_error)?;
        if result.rows_affected() != 1 {
            return Err(Error::Conflict);
        }
        Ok(())
    }
}

impl PostgresCatalogPreviews {
    async fn authorized_read(
        &self,
        user: UserId,
        project: ProjectId,
    ) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        // Same user-row lock used by permission snapshot replacement: grants cannot change
        // between authorization and returning a candidate belonging to this project.
        let row = sqlx::query(
            r#"SELECT u.enabled,u.grants_synced,EXISTS(SELECT 1
            FROM user_project_access a
            WHERE a.user_id=u.id
            AND a.project_id=$2) AS allowed
            FROM users u
            JOIN projects p ON p.instance=u.instance
            WHERE u.id=$1
            AND p.id=$2 FOR SHARE OF u"#,
        )
        .bind(uuid(user))
        .bind(uuid(project))
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?
        .ok_or(Error::NotFound)?;
        if !row.get::<bool, _>("enabled") {
            return Err(Error::Unauthenticated);
        }
        if !row.get::<bool, _>("grants_synced") {
            return Err(Error::Unavailable {
                component: "project_access",
            });
        }
        if !row.get::<bool, _>("allowed") {
            return Err(Error::Forbidden);
        }
        Ok(tx)
    }
}
#[async_trait]
impl nexofolio_knowledge::CatalogPreviewReader for PostgresCatalogPreviews {
    async fn list_previews(
        &self,
        user: UserId,
        project: ProjectId,
        page: u32,
        limit: u32,
        status: Option<PreviewStatus>,
    ) -> Result<CatalogPreviewPage> {
        if !(1..=100000).contains(&page) || !(1..=100).contains(&limit) {
            return Err(Error::InvalidInput {
                message: "invalid catalog preview pagination".into(),
            });
        }
        let mut tx = self.authorized_read(user, project).await?;
        let status = status.map(|s| {
            serde_json::to_value(s)
                .expect("serializes")
                .as_str()
                .expect("enum string")
                .to_owned()
        });
        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM catalog_preview_tasks WHERE project_id=$1 AND ($2::text IS NULL OR status=$2)")
                .bind(uuid(project))
                .bind(&status)
                .fetch_one(&mut *tx)
                .await
                .map_err(db_error)?;
        let rows=sqlx::query(r#"SELECT jsonb_build_object('task_id',id,'candidate_id',candidate_id,'status',status,'snapshot_at',snapshot_at,'interface_count',jsonb_array_length(snapshot->'interfaces'),'directory_count',review->'metrics'->'directory_count','unclassified_count',review->'metrics'->'unclassified_interfaces','structurally_valid',review->'structurally_valid') AS value
            FROM catalog_preview_tasks
            WHERE project_id=$1
            AND ($4::text IS NULL
            OR status=$4)
            ORDER BY snapshot_at DESC,id DESC
            LIMIT $2 OFFSET $3"#)
            .bind(uuid(project)).bind(i64::from(limit)).bind(i64::from(page-1)*i64::from(limit)).bind(&status).fetch_all(&mut *tx).await.map_err(db_error)?;
        let items = rows
            .into_iter()
            .map(|r| {
                serde_json::from_value(r.get::<Value, _>("value")).map_err(|_| Error::Unavailable {
                    component: "catalog_preview_contract",
                })
            })
            .collect::<Result<Vec<_>>>()?;
        tx.commit().await.map_err(db_error)?;
        Ok(CatalogPreviewPage {
            items,
            total,
            page,
            limit,
        })
    }
    async fn preview_detail(
        &self,
        user: UserId,
        project: ProjectId,
        task: JobId,
    ) -> Result<CatalogPreviewDetail> {
        let mut tx = self.authorized_read(user, project).await?;
        let row = sqlx::query(READ_PROJECT)
            .bind(uuid(task))
            .bind(uuid(project))
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_error)?
            .ok_or(Error::NotFound)?;
        let task = decode(row)?;
        let published:bool=sqlx::query_scalar("SELECT coalesce(current_version_id=$2,false) FROM project_catalogs WHERE project_id=$1").bind(uuid(project)).bind(uuid(task.candidate_id)).fetch_one(&mut *tx).await.map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(CatalogPreviewDetail { published, task })
    }
}
