//! Background-only complete database aggregates; one scalar row, 350ms per statement.
use super::PostgresObserve;
use nexofolio_common::{
    Error, ProjectId, Result,
    storage::{StorageGroup, StorageReader, StorageTable, StorageUsage},
};
use sqlx::Row;
fn unavailable<E>(_: E) -> Error {
    Error::Unavailable {
        component: "storage_sample",
    }
}
#[async_trait::async_trait]
impl StorageReader for PostgresObserve {
    async fn storage_usage(&self, project: ProjectId) -> Result<StorageUsage> {
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        sqlx::query("SELECT set_config('statement_timeout','350ms',true), set_config('work_mem','1MB',true), set_config('max_parallel_workers_per_gather','0',true)")
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        let mut groups = Vec::new();
        for (name, source) in [
            (
                "原始资料 · 样本与声明",
                "SELECT count(*)::bigint AS rows, coalesce(sum(bytes),0)::bigint AS bytes FROM (SELECT coalesce(pg_column_size(f.sample),0)::bigint+coalesce(pg_column_size(f.structure),0) AS bytes FROM fingerprints f JOIN endpoints e ON e.id=f.endpoint_id WHERE e.project_id=$1 UNION ALL SELECT pg_column_size(d.structure)::bigint FROM declarations d JOIN endpoints e ON e.id=d.endpoint_id WHERE e.project_id=$1) contents",
            ),
            (
                "观测值与证据 · 代表值",
                "SELECT count(*)::bigint AS rows, coalesce(sum(bytes),0)::bigint AS bytes FROM (SELECT pg_column_size(f.representatives)::bigint AS bytes FROM fingerprints f JOIN endpoints e ON e.id=f.endpoint_id WHERE e.project_id=$1) contents",
            ),
        ] {
            let row = sqlx::query(source)
                .bind(
                    project
                        .to_string()
                        .parse::<uuid::Uuid>()
                        .map_err(unavailable)?,
                )
                .fetch_one(&mut *tx)
                .await
                .map_err(unavailable)?;
            groups.push(StorageGroup {
                name: name.into(),
                rows: Some(row.get("rows")),
                column_bytes: Some(row.get("bytes")),
                logical_bytes: None,
            });
        }
        let mut tables = Vec::new();
        for name in ["fingerprints", "declarations"] {
            let row = sqlx::query("SELECT pg_table_size($1::regclass) AS tables, pg_indexes_size($1::regclass) AS indexes").bind(name).fetch_one(&mut *tx).await.map_err(unavailable)?;
            tables.push(StorageTable {
                name: format!("observe.{name}"),
                table_bytes: row.get("tables"),
                index_bytes: row.get("indexes"),
            });
        }
        Ok(StorageUsage { groups, tables })
    }
}
