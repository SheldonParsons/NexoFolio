//! Historical candidate reader only: no creation, model calls, leases or execution writes.
use crate::Postgres;
use async_trait::async_trait;
use nexofolio_contracts::*;
use serde_json::Value;
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
fn decode(row: sqlx::postgres::PgRow) -> Result<PreviewTask> {
    serde_json::from_value(row.get::<Value, _>("value")).map_err(|_| Error::Unavailable {
        component: "catalog_preview_contract",
    })
}
const READ: &str = "SELECT jsonb_build_object('task_id',id,'candidate_id',candidate_id,'contract_version',contract_version,'status',status,'snapshot_at',snapshot_at,'snapshot_sha256',snapshot_sha256,'snapshot',snapshot,'generation',generation,'generator',generator,'error_code',error_code,'candidate',candidate,'review',review) AS value FROM catalog_preview_tasks WHERE id=$1";
const READ_PROJECT: &str = "SELECT jsonb_build_object('task_id',id,'candidate_id',candidate_id,'contract_version',contract_version,'status',status,'snapshot_at',snapshot_at,'snapshot_sha256',snapshot_sha256,'snapshot',snapshot,'generation',generation,'generator',generator,'error_code',error_code,'candidate',candidate,'review',review) AS value FROM catalog_preview_tasks WHERE id=$1 AND project_id=$2";
impl PostgresCatalogPreviews {
    pub async fn read(&self, task: JobId) -> Result<PreviewTask> {
        decode(
            sqlx::query(READ)
                .bind(uuid(task))
                .fetch_optional(&self.database.pool)
                .await
                .map_err(db_error)?
                .ok_or(Error::NotFound)?,
        )
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
        crate::project_access::authorize_project(&mut tx, user, project).await?;
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
            return Err(Error::invalid("invalid catalog preview pagination"));
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
