//! Bounded keyset reads and background-only counts, owned by knowledge.
use super::{
    PostgresKnowledge,
    codec::{id, parse, text, unavailable, uuid},
};
use async_trait::async_trait;
use chrono::Utc;
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::{TaskState, safe_task_reason};
use nexofolio_knowledge_contracts::{StoreResult, operations::*};
use sqlx::{QueryBuilder, Row};

impl PostgresKnowledge {
    async fn operations_read(&self) -> StoreResult<sqlx::Transaction<'_, sqlx::Postgres>> {
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        sqlx::query("SET LOCAL statement_timeout='1500ms'")
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        Ok(tx)
    }
}
#[async_trait]
impl OperationsReader for PostgresKnowledge {
    async fn operations_tasks(&self, f: &TaskFilter) -> StoreResult<TaskPage> {
        let limit = f.limit.clamp(1, 50) as usize;
        let mut tx = self.operations_read().await?;
        let mut query = QueryBuilder::new(
            "WITH page AS (SELECT id,project_id,candidate_id,target,state,run_id,generation,created_at,updated_at,lease_until,next_retry_at,retry_reason,retry_count FROM curation_tasks WHERE true",
        );
        if let Some(project) = f.project {
            query.push(" AND project_id=").push_bind(uuid(project));
        }
        if let Some(state) = f.state {
            query.push(" AND state=").push_bind(text(state));
        }
        if let Some(cursor) = &f.before {
            query
                .push(" AND (created_at,id)<(")
                .push_bind(cursor.created_at)
                .push(",")
                .push_bind(cursor.id)
                .push(")");
        }
        query
            .push(" ORDER BY created_at DESC,id DESC LIMIT ")
            .push_bind((limit + 1) as i64);
        // Reading a JSONB scalar can detoast its entire source document. Poll only the
        // bounded write-time summary, never progress, batches, snapshots or frozen input.
        query.push(") SELECT p.*, c.updated_at AS checkpoint_at, c.operations_checkpoint AS checkpoint FROM page p LEFT JOIN candidates c ON c.id=p.candidate_id AND c.project_id=p.project_id ORDER BY p.created_at DESC,p.id DESC");
        let rows = query
            .build()
            .fetch_all(&mut *tx)
            .await
            .map_err(unavailable)?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let p = row
                .get::<Option<serde_json::Value>, _>("checkpoint")
                .unwrap_or(serde_json::Value::Null);
            let checkpoint = row
                .get::<Option<chrono::DateTime<Utc>>, _>("checkpoint_at")
                .filter(|_| p.is_object())
                .map(|_| TaskCheckpoint {
                    run_id: p["run_id"].as_str().and_then(|s| s.parse().ok()),
                    stage: p["stage"]
                        .as_str()
                        .filter(|s| matches!(*s, "describe" | "fields" | "organize"))
                        .map(str::to_owned),
                    completed: p["completed"].as_u64(),
                    total: p["total"].as_u64(),
                    completed_batches: p["completed_batches"].as_u64(),
                    saved_batch_records: None,
                    updated_at: p["updated_at"]
                        .as_str()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|t| t.with_timezone(&Utc)),
                });
            let state: TaskState = parse(row.get("state"))?;
            let run_id: Option<uuid::Uuid> = row.get("run_id");
            let current = checkpoint
                .as_ref()
                .filter(|p| state == TaskState::Running && run_id.is_some() && p.run_id == run_id);
            items.push(OperationsTask {
                id: row.get("id"),
                project_id: id(row.get("project_id")),
                candidate_id: row.get("candidate_id"),
                target: row.get::<Option<uuid::Uuid>, _>("target").map(id),
                state,
                run_id,
                generation: row.get("generation"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                lease_until: row.get("lease_until"),
                next_retry_at: row.get("next_retry_at"),
                retry_reason: safe_task_reason(row.get::<Option<&str>, _>("retry_reason"))
                    .map(str::to_owned),
                retry_count: row.get("retry_count"),
                stage: current.and_then(|p| p.stage.clone()),
                heartbeat_at: current.and_then(|_| row.get("checkpoint_at")),
                checkpoint,
            });
        }
        let more = items.len() > limit;
        items.truncate(limit);
        let next = items.last().filter(|_| more).map(|t| TaskCursor {
            created_at: t.created_at,
            id: t.id,
        });
        tx.commit().await.map_err(unavailable)?;
        Ok(TaskPage {
            items,
            next,
            observed_at: Utc::now(),
        })
    }
    async fn operations_counts(&self) -> StoreResult<Vec<TaskCount>> {
        let mut tx = self.operations_read().await?;
        let rows = sqlx::query("SELECT state,count(*) AS count FROM curation_tasks GROUP BY state")
            .fetch_all(&mut *tx)
            .await
            .map_err(unavailable)?;
        let counts = rows
            .iter()
            .map(|r| {
                Ok(TaskCount {
                    state: parse(r.get("state"))?,
                    count: r.get("count"),
                })
            })
            .collect::<StoreResult<_>>()?;
        tx.commit().await.map_err(unavailable)?;
        Ok(counts)
    }
    async fn task_project(&self, task: uuid::Uuid) -> StoreResult<Option<ProjectId>> {
        sqlx::query_scalar("SELECT project_id FROM curation_tasks WHERE id=$1")
            .bind(task)
            .fetch_optional(&self.pool)
            .await
            .map(|v| v.map(id))
            .map_err(unavailable)
    }
}
