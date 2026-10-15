//! SQL projects only the requested review scope, never a complete Candidate.
use super::{
    codec::{unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::{CandidateReviewQuery, ReviewPart};
use nexofolio_knowledge_contracts::{StoreError, StoreResult};
use serde_json::Value;
use sqlx::Row;

impl PostgresTx {
    pub(super) async fn db_review_field_notes(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
        endpoint: nexofolio_common::EndpointId,
        keys: &[String],
    ) -> StoreResult<Vec<nexofolio_contracts::knowledge::FieldNote>> {
        let value: sqlx::types::Json<Vec<nexofolio_contracts::knowledge::FieldNote>> = sqlx::query_scalar("SELECT coalesce(jsonb_agg(f),'[]'::jsonb) FROM candidates c CROSS JOIN LATERAL jsonb_array_elements(c.view->'fields') f WHERE c.project_id=$1 AND c.id=$2 AND f->>'endpoint'=$3 AND f->>'path'=ANY($4)")
            .bind(uuid(project)).bind(id).bind(endpoint.to_string()).bind(keys).fetch_one(&mut *self.tx).await.map_err(unavailable)?;
        Ok(value.0)
    }

    pub(super) async fn db_review_candidate(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
        q: &CandidateReviewQuery,
    ) -> StoreResult<Option<Value>> {
        // This read shares a repeatable-read snapshot with the scoped body below.
        let row = sqlx::query("SELECT review_version,history_pruned_at IS NOT NULL AS expired,jsonb_build_object('id',id,'project_id',project_id,'target',target,'parent',parent,'state',state,'created_at',created_at,'updated_at',updated_at,'version',review_version,'progress',progress,'resume_available',execution_input IS NOT NULL AND execution_released_at IS NULL AND coalesce(progress->>'recovery_blocked','false')!='true' AND state IN ('running','partial','interrupted'),'history_available',history_pruned_at IS NULL,'applied_rounds',applied_rounds) AS status FROM candidates WHERE project_id=$1 AND id=$2")
            .bind(uuid(project)).bind(id).fetch_optional(&mut *self.tx).await.map_err(unavailable)?;
        let Some(row) = row else { return Ok(None) };
        let mut status: Value = row.get("status");
        if matches!(q.part, ReviewPart::Status) {
            return Ok(Some(status));
        }
        if row.get::<bool, _>("expired") {
            return Err(StoreError::HistoryExpired);
        }
        // Caller maps a version mismatch to a conflict, never merges pages across content versions.
        if q.version
            .is_some_and(|v| v != row.get::<i64, _>("review_version"))
        {
            status["version_mismatch"] = Value::Bool(true);
            return Ok(Some(status));
        }
        let sql = match q.part {
            ReviewPart::Head => include_str!("review_head.sql"),
            ReviewPart::Changes => {
                r#"SELECT jsonb_build_object('items',coalesce(jsonb_agg(jsonb_build_object('key',key,'before',before,'after',after) ORDER BY ordinal),'[]')) FROM (
                SELECT d.* FROM candidates c CROSS JOIN LATERAL review_diff(c.base,c.view,$3,$6) d
                WHERE c.project_id=$1 AND c.id=$2 AND ($4::text IS NULL OR d.key=$4)
                ORDER BY ordinal LIMIT 60 OFFSET $5) page"#
            }
            ReviewPart::Endpoint => include_str!("review_endpoint.sql"),
            ReviewPart::Status => unreachable!(),
        };
        let defaults = review_defaults();
        let body = match q.part {
            ReviewPart::Head => {
                sqlx::query_scalar::<_, Value>(sql)
                    .bind(uuid(project))
                    .bind(id)
                    .bind(&defaults)
                    .fetch_one(&mut *self.tx)
                    .await
            }
            ReviewPart::Changes => {
                sqlx::query_scalar::<_, Value>(sql)
                    .bind(uuid(project))
                    .bind(id)
                    .bind(q.section.as_deref().unwrap_or("notes"))
                    .bind(q.key.as_deref())
                    .bind(q.offset as i64)
                    .bind(defaults[q.section.as_deref().unwrap_or("notes")].clone())
                    .fetch_one(&mut *self.tx)
                    .await
            }
            ReviewPart::Endpoint => {
                sqlx::query_scalar::<_, Value>(sql)
                    .bind(uuid(project))
                    .bind(id)
                    .bind(q.endpoint.map(|e| e.to_string()))
                    .bind(q.offset as i64)
                    .fetch_one(&mut *self.tx)
                    .await
            }
            ReviewPart::Status => unreachable!(),
        }
        .map_err(unavailable)?;
        status
            .as_object_mut()
            .ok_or(StoreError::Unavailable)?
            .extend(body.as_object().ok_or(StoreError::Unavailable)?.clone());
        Ok(Some(status))
    }
}

fn review_defaults() -> Value {
    use nexofolio_contracts::knowledge::{EndpointContext, FieldContext, LinkContext};
    let mut field = serde_json::to_value(FieldContext::default()).expect("field context");
    field["evidence"] = serde_json::json!([{"options":[],"source":null}]);
    serde_json::json!({"folders":{"author":null},"placements":{},
        "notes":{"context":EndpointContext::default()},
        "fields":{"values":[],"context":field},"links":{"context":LinkContext::default()}})
}
