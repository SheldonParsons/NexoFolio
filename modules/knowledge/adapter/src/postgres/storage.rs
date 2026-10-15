//! Background-only complete database aggregates; one scalar row, 350ms per statement.
use super::PostgresKnowledge;
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
impl StorageReader for PostgresKnowledge {
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
                "运行快照 · 执行头与冻结块",
                "SELECT count(*)::bigint AS rows, coalesce(sum(bytes),0)::bigint AS bytes FROM (SELECT pg_column_size(execution_input)::bigint AS bytes FROM candidates WHERE project_id=$1 UNION ALL SELECT pg_column_size(content)::bigint FROM frozen_input_chunks WHERE project_id=$1) contents",
            ),
            (
                "观测值与证据 · 归档及证据头",
                "SELECT count(*)::bigint AS rows, coalesce(sum(bytes),0)::bigint AS bytes FROM (SELECT pg_column_size(archive)::bigint AS bytes FROM field_value_archives WHERE project_id=$1 UNION ALL SELECT pg_column_size(evidence_input)::bigint FROM candidates WHERE project_id=$1) contents",
            ),
            (
                "整理历史 · 候选与回滚",
                "SELECT count(*)::bigint AS rows, coalesce(sum(bytes),0)::bigint AS bytes FROM (SELECT coalesce(pg_column_size(base),0)::bigint+coalesce(pg_column_size(view),0)+coalesce(pg_column_size(batches),0) AS bytes FROM candidates WHERE project_id=$1 UNION ALL SELECT coalesce(pg_column_size(v.target),0)::bigint+coalesce(pg_column_size(v.before),0) FROM revisions v JOIN rounds r ON r.id=v.round_id WHERE r.project_id=$1) contents",
            ),
            (
                "正式知识 · 字段与接口说明",
                "SELECT count(*)::bigint AS rows, coalesce(sum(bytes),0)::bigint AS bytes FROM (SELECT coalesce(pg_column_size(text),0)::bigint+coalesce(pg_column_size(context),0)+coalesce(pg_column_size(values_seen),0) AS bytes FROM field_notes WHERE project_id=$1 UNION ALL SELECT coalesce(pg_column_size(name),0)::bigint+coalesce(pg_column_size(purpose),0) FROM endpoint_notes WHERE project_id=$1) contents",
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
        for name in [
            "candidates",
            "frozen_input_chunks",
            "field_value_archives",
            "revisions",
            "rounds",
            "field_notes",
            "endpoint_notes",
        ] {
            let row = sqlx::query("SELECT pg_table_size($1::regclass) AS tables, pg_indexes_size($1::regclass) AS indexes").bind(name).fetch_one(&mut *tx).await.map_err(unavailable)?;
            tables.push(StorageTable {
                name: format!("knowledge.{name}"),
                table_bytes: row.get("tables"),
                index_bytes: row.get("indexes"),
            });
        }
        Ok(StorageUsage { groups, tables })
    }
}

#[async_trait::async_trait]
impl nexofolio_knowledge_contracts::retention::HistoryMaintenance for PostgresKnowledge {
    async fn retention_status(
        &self,
        project: ProjectId,
    ) -> nexofolio_knowledge_contracts::StoreResult<
        nexofolio_knowledge_contracts::retention::RetentionStatus,
    > {
        use nexofolio_knowledge_contracts::{StoreError, retention::RetentionStatus};
        let read = async {
            let mut tx = self.pool.begin().await?;
            sqlx::query("SET TRANSACTION READ ONLY")
                .execute(&mut *tx)
                .await?;
            sqlx::query("SET LOCAL statement_timeout='350ms'")
                .execute(&mut *tx)
                .await?;
            let id = project
                .to_string()
                .parse::<uuid::Uuid>()
                .expect("typed project UUID");
            let enabled_at: Option<chrono::DateTime<chrono::Utc>> =
                sqlx::query_scalar("SELECT enabled_at FROM retention_projects WHERE project_id=$1")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let mut stopped: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT t.id FROM curation_tasks t JOIN candidates c ON c.id=t.candidate_id AND c.project_id=t.project_id WHERE t.project_id=$1 AND t.state='stopped' AND c.state NOT IN ('applied','discarded') ORDER BY t.id LIMIT 21").bind(id).fetch_all(&mut *tx).await?;
            let more_stopped = stopped.len() > 20;
            stopped.truncate(20);
            Ok::<_, sqlx::Error>(RetentionStatus {
                enabled_at,
                protected_until: enabled_at.map(|at| at + chrono::Duration::days(7)),
                stopped_recoverable: stopped,
                more_stopped,
            })
        };
        read.await.map_err(|_| StoreError::Unavailable)
    }
    async fn maintain(
        &self,
        project: ProjectId,
        after: Option<uuid::Uuid>,
        execute: bool,
    ) -> nexofolio_knowledge_contracts::StoreResult<super::RetentionReport> {
        self.retain_history(project, chrono::Utc::now(), after, 10, execute)
            .await
    }
}
