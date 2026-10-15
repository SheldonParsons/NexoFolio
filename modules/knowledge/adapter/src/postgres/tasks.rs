use super::{
    PROJECT_LOCK,
    codec::{id, json, parse, text, unavailable, uuid},
    transaction::PostgresTx,
};
use chrono::{DateTime, Utc};
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::CurationTask;
use nexofolio_knowledge_contracts::StoreResult;
use sqlx::{Row, postgres::PgRow};
fn task(r: &PgRow) -> StoreResult<CurationTask> {
    Ok(CurationTask {
        id: r.get("id"),
        project_id: id(r.get("project_id")),
        target: r.get::<Option<uuid::Uuid>, _>("target").map(id),
        parent: r.get("parent"),
        candidate_id: r.get("candidate_id"),
        actor: serde_json::from_value(r.get("actor")).map_err(unavailable)?,
        state: parse(r.get("state"))?,
        generation: r.get("generation"),
        run_id: r.get("run_id"),
        lease_until: r.get("lease_until"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
        last_started_at: r.get("last_started_at"),
        message: r.get("message"),
        next_retry_at: r.get("next_retry_at"),
        retry_reason: r.get("retry_reason"),
        retry_count: r.get::<i64, _>("retry_count") as u32,
        timing: serde_json::from_value(r.get("timing")).map_err(unavailable)?,
    })
}
impl PostgresTx {
    pub(super) async fn db_candidate_task(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
    ) -> StoreResult<Option<CurationTask>> {
        sqlx::query("SELECT * FROM curation_tasks WHERE project_id=$1 AND candidate_id=$2")
            .bind(uuid(project))
            .bind(candidate)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(unavailable)?
            .as_ref()
            .map(task)
            .transpose()
    }

    pub(super) async fn db_task(
        &mut self,
        project: ProjectId,
        key: uuid::Uuid,
    ) -> StoreResult<Option<CurationTask>> {
        sqlx::query("SELECT * FROM curation_tasks WHERE project_id=$1 AND id=$2")
            .bind(uuid(project))
            .bind(key)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(unavailable)?
            .as_ref()
            .map(task)
            .transpose()
    }
    pub(super) async fn db_active_tasks(
        &mut self,
        project: ProjectId,
    ) -> StoreResult<Vec<CurationTask>> {
        sqlx::query("SELECT * FROM curation_tasks WHERE project_id=$1 AND state IN ('queued','running','waiting','needs_attention') ORDER BY (state='running') DESC,created_at,id").bind(uuid(project)).fetch_all(&mut *self.tx).await.map_err(unavailable)?.iter().map(task).collect()
    }
    pub(super) async fn db_put_task(&mut self, t: &CurationTask) -> StoreResult<()> {
        sqlx::query("INSERT INTO curation_tasks(id,project_id,target,parent,candidate_id,actor,state,generation,run_id,lease_until,created_at,updated_at,last_started_at,message,next_retry_at,retry_reason,retry_count,timing) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18) ON CONFLICT(id) DO UPDATE SET candidate_id=EXCLUDED.candidate_id,actor=EXCLUDED.actor,state=EXCLUDED.state,generation=EXCLUDED.generation,run_id=EXCLUDED.run_id,lease_until=EXCLUDED.lease_until,updated_at=EXCLUDED.updated_at,last_started_at=EXCLUDED.last_started_at,message=EXCLUDED.message,next_retry_at=EXCLUDED.next_retry_at,retry_reason=EXCLUDED.retry_reason,retry_count=EXCLUDED.retry_count,timing=EXCLUDED.timing")
        .bind(t.id).bind(uuid(t.project_id)).bind(t.target.map(uuid)).bind(t.parent).bind(t.candidate_id).bind(json(&t.actor)?).bind(text(t.state)).bind(t.generation).bind(t.run_id).bind(t.lease_until).bind(t.created_at).bind(t.updated_at).bind(t.last_started_at).bind(&t.message).bind(t.next_retry_at).bind(&t.retry_reason).bind(i64::from(t.retry_count)).bind(json(&t.timing)?).execute(&mut *self.tx).await.map_err(unavailable)?;
        Ok(())
    }
    pub(super) async fn db_next_task(
        &mut self,
        now: DateTime<Utc>,
        capacity: usize,
    ) -> StoreResult<Option<CurationTask>> {
        // A short scheduler transaction; never held across input preparation or model IO.
        sqlx::query("SELECT pg_advisory_xact_lock($1,0)")
            .bind(PROJECT_LOCK + 1)
            .execute(&mut *self.tx)
            .await
            .map_err(unavailable)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM curation_tasks WHERE state='running' AND lease_until>$1",
        )
        .bind(now)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        if count >= capacity as i64 {
            return Ok(None);
        }
        let row=sqlx::query("SELECT t.* FROM curation_tasks t WHERE (t.state='queued' OR (t.state='waiting' AND t.next_retry_at<=$1) OR (t.state='running' AND t.lease_until<=$1)) AND NOT EXISTS(SELECT 1 FROM curation_tasks r WHERE r.project_id=t.project_id AND r.state IN ('running','waiting','needs_attention') AND r.id<>t.id) ORDER BY (SELECT h.last_started_at FROM curation_tasks h WHERE h.project_id=t.project_id ORDER BY h.last_started_at DESC NULLS LAST LIMIT 1) ASC NULLS FIRST, t.created_at,t.id LIMIT 1")
            .bind(now).fetch_optional(&mut *self.tx).await.map_err(unavailable)?;
        let Some(t) = row.as_ref().map(task).transpose()? else {
            return Ok(None);
        };
        sqlx::query("SELECT pg_advisory_xact_lock($1,hashtext($2))")
            .bind(PROJECT_LOCK)
            .bind(t.project_id.to_string())
            .execute(&mut *self.tx)
            .await
            .map_err(unavailable)?;
        // A stop, heartbeat or content reset may have won while obtaining the lock.
        Ok(self.db_task(t.project_id, t.id).await?.filter(|t| {
            t.state == nexofolio_contracts::knowledge::TaskState::Queued
                || (t.state == nexofolio_contracts::knowledge::TaskState::Waiting
                    && t.next_retry_at.is_some_and(|until| until <= now))
                || (t.state == nexofolio_contracts::knowledge::TaskState::Running
                    && t.lease_until.is_some_and(|until| until <= now))
        }))
    }
    pub(super) async fn db_valid_worker(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
        owner: Option<uuid::Uuid>,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        Ok(sqlx::query_scalar::<_,bool>("SELECT state='running' AND run_id=$3 AND lease_until>$4 FROM curation_tasks WHERE project_id=$1 AND candidate_id=$2").bind(uuid(project)).bind(candidate).bind(owner).bind(now).fetch_optional(&mut *self.tx).await.map_err(unavailable)?.unwrap_or(true))
    }
    pub(super) async fn db_close_candidate_tasks(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
        now: DateTime<Utc>,
    ) -> StoreResult<()> {
        for mut task in self.db_active_tasks(project).await? {
            if task.candidate_id != Some(candidate) && task.parent != Some(candidate) {
                continue;
            }
            task.transition(nexofolio_contracts::knowledge::TaskState::Stopped, now);
            task.lease_until = None;
            task.message = Some("候选已停止、应用或废弃".into());
            self.db_put_task(&task).await?;
            sqlx::query("UPDATE candidates SET state=CASE WHEN state='running' THEN 'interrupted' ELSE state END,updated_at=$3,progress=(CASE WHEN jsonb_typeof(progress)='object' THEN progress ELSE '{}'::jsonb END)||jsonb_build_object('status','failed','task_state','stopped','candidate_state',CASE WHEN state='running' THEN 'interrupted' ELSE state END,'active','[]'::jsonb,'message','关联候选已关闭，任务已停止；已保存批次保留') WHERE project_id=$1 AND state IN ('running','partial') AND id=$2")
                .bind(uuid(project)).bind(task.candidate_id).bind(now).execute(&mut *self.tx).await.map_err(unavailable)?;
        }
        Ok(())
    }
}
