use super::{PostgresAudit, unavailable};
use nexofolio_audit_contracts::{AccessQuery, Event, Limits};
use nexofolio_common::Result;
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};

/// System/supporting queries are not page visits; this does not infer manual vs automatic origin.
fn background(event: &Event) -> bool {
    event.kind == "api"
        && (event.route.starts_with("/mcp")
            || event.route.starts_with("/v1/auth/")
            || event.method == "GET"
                && (event.route.contains("/curate")
                    || matches!(event.route.as_str(), "/v1/sites/lookup" | "/v1/downloads")))
}
pub(super) async fn write(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    events: &[Event],
    limits: &Limits,
) -> Result<()> {
    let access: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.kind.as_str(), "page" | "api" | "mcp"))
        .map(|e| json!({"event":e,"background":background(e)}))
        .collect();
    sqlx::query("WITH added AS (INSERT INTO access_events(at,kind,actor,project,resource,route,action,method,background,data) SELECT (v->'event'->>'at')::timestamptz,v->'event'->>'kind',v->'event'->>'actor',v->'event'->>'project',v->'event'->>'resource',v->'event'->>'route',v->'event'->>'action',v->'event'->>'method',(v->>'background')::boolean,v->'event' FROM jsonb_array_elements($1::jsonb) v RETURNING id) UPDATE counters SET access_events=access_events+(SELECT count(*) FROM added) WHERE id=true")
        .bind(json!(access)).execute(&mut **tx).await.map_err(unavailable)?;
    sqlx::query("WITH removed AS (DELETE FROM access_events WHERE id IN (SELECT id FROM access_events ORDER BY at,id LIMIT (SELECT greatest(access_events-$1,0) FROM counters)) RETURNING id) UPDATE counters SET access_events=access_events-(SELECT count(*) FROM removed),evicted_access_events=evicted_access_events+(SELECT count(*) FROM removed) WHERE id=true")
        .bind(i64::from(limits.events)).execute(&mut **tx).await.map_err(unavailable)?;
    Ok(())
}
pub(super) async fn purge(tx: &mut sqlx::Transaction<'_, Postgres>, limits: &Limits) -> Result<()> {
    sqlx::query("WITH removed AS (DELETE FROM access_events WHERE id IN (SELECT id FROM access_events WHERE at<now()-($1*interval '1 day') ORDER BY at,id LIMIT 5000) RETURNING id) UPDATE counters SET access_events=access_events-(SELECT count(*) FROM removed) WHERE id=true")
        .bind(i32::try_from(limits.access_days).unwrap_or(30)).execute(&mut **tx).await.map_err(unavailable)?;
    Ok(())
}
const SUPPORT_QUERY_SQL: &str = "kind='api' AND (route LIKE '/mcp%' OR route LIKE '/v1/auth/%' OR (method='GET' AND (route LIKE '%/curate%' OR route IN ('/v1/sites/lookup','/v1/downloads'))))";
const PAGE_GROUP_SQL: &str = "CASE WHEN route='home' THEN 'home' WHEN route IN ('projects','project-interfaces') THEN 'projects' WHEN route IN ('document-folder','document-endpoint','product-docs') OR route LIKE 'product-docs:%' THEN 'docs' ELSE 'other' END";
fn conditions(q: &AccessQuery, sql: &mut QueryBuilder<Postgres>) {
    sql.push(" WHERE at >= ")
        .push_bind(q.since)
        .push(" AND at <= ")
        .push_bind(q.until)
        .push(" AND kind=")
        .push_bind(&q.kind);
    for (column, value) in [
        ("project", &q.project),
        ("route", &q.route),
        ("resource", &q.resource),
        ("action", &q.action),
        ("method", &q.method),
        ("actor", &q.actor),
    ] {
        if let Some(value) = value {
            sql.push(" AND ").push(column).push("=").push_bind(value);
        }
    }
    if let Some(group) = &q.page_group {
        sql.push(" AND (")
            .push(PAGE_GROUP_SQL)
            .push(")=")
            .push_bind(group);
    }
    if !q.background {
        sql.push(" AND NOT background AND NOT (")
            .push(SUPPORT_QUERY_SQL)
            .push(")");
    }
}
impl PostgresAudit {
    pub(super) async fn read_access(&self, q: &AccessQuery) -> Result<Value> {
        q.validate()?;
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        let coverage:Value=sqlx::query_scalar("SELECT jsonb_build_object('detail_recording_started_at',access_details_since,'retained_from',(SELECT min(at) FROM access_events),'retained_until',(SELECT max(at) FROM access_events),'stored_details',access_events,'capacity_evictions',evicted_access_events,'hourly_buckets',buckets,'hourly_evictions',evicted_buckets) FROM counters WHERE id=true").fetch_one(&mut *tx).await.map_err(unavailable)?;
        let value = if q.view == "legacy" {
            // Hourly evidence is kept separate, never expanded into invented individual visits.
            let mut sql = QueryBuilder::new("SELECT to_jsonb(b) FROM buckets b WHERE kind=");
            sql.push_bind(&q.kind).push(" AND hour>=date_trunc('hour',").push_bind(q.since).push("::timestamptz) AND hour<=").push_bind(q.until).push(" AND hour<date_trunc('hour',(SELECT access_details_since FROM counters WHERE id=true))");
            if let Some(project) = &q.project {
                sql.push(" AND project=").push_bind(project);
            }
            if let Some(group) = &q.page_group {
                sql.push(" AND (")
                    .push(PAGE_GROUP_SQL)
                    .push(")=")
                    .push_bind(group);
            }
            if !q.background {
                sql.push(" AND NOT (").push(SUPPORT_QUERY_SQL).push(")");
            }
            sql.push(" ORDER BY hour DESC,key LIMIT ")
                .push_bind(i64::from(q.limit) + 1)
                .push(" OFFSET ")
                .push_bind(i64::from(q.page - 1) * i64::from(q.limit));
            let rows: Vec<Value> = sql
                .build_query_scalar::<Value>()
                .fetch_all(&mut *tx)
                .await
                .map_err(unavailable)?;
            json!({"items":rows.iter().take(q.limit as usize).collect::<Vec<_>>(),"has_more":rows.len()>q.limit as usize,"precision":"hour","coverage":coverage,"page":q.page,"limit":q.limit})
        } else {
            // Execute aggregation and page count on all retained matching rows, never on a limited sample.
            let prefix = if q.view == "ranking" {
                "SELECT kind,project,resource,route,action,method,count(*) AS hits,count(DISTINCT actor) FILTER(WHERE actor LIKE 'user:%') AS users,count(DISTINCT actor) FILTER(WHERE actor LIKE 'ip:%') AS visitor_ips,count(*) FILTER(WHERE actor='unknown') AS unknown_visits,max(at) AS last_at FROM selected GROUP BY kind,project,resource,route,action,method"
            } else {
                "SELECT id,data,at FROM selected"
            };
            let mut sql = QueryBuilder::new("WITH selected AS (SELECT * FROM access_events");
            conditions(q, &mut sql);
            sql.push("), result AS (")
                .push(prefix)
                .push("), page AS (SELECT * FROM result ORDER BY ");
            sql.push(if q.view == "ranking" {
                "hits DESC,last_at DESC,project,resource,route,action,method"
            } else {
                "at DESC,id DESC"
            })
            .push(" LIMIT ")
            .push_bind(i64::from(q.limit))
            .push(" OFFSET ")
            .push_bind(i64::from(q.page - 1) * i64::from(q.limit));
            sql.push(") SELECT jsonb_build_object('items',coalesce((SELECT jsonb_agg(").push(if q.view=="ranking" {"to_jsonb(page)"} else {"data || jsonb_build_object('id',id)"}).push(") FROM page),'[]'::jsonb),'total',(SELECT count(*) FROM result),'matching_visits',(SELECT count(*) FROM selected))");
            let mut result: Value = sql
                .build_query_scalar::<Value>()
                .fetch_one(&mut *tx)
                .await
                .map_err(unavailable)?;
            result["coverage"] = coverage;
            result["page"] = json!(q.page);
            result["limit"] = json!(q.limit);
            result["precision"] = json!("event");
            result
        };
        tx.commit().await.map_err(unavailable)?;
        Ok(value)
    }
    pub(super) async fn read_candidate_events(
        &self,
        project: &str,
        candidate: Option<&str>,
        task: Option<&str>,
    ) -> Result<Value> {
        let value:Value=sqlx::query_scalar("WITH selected AS (SELECT * FROM events WHERE project=$1 AND (($2::text IS NOT NULL AND data->>'resource'=$2 AND ($3::text IS NULL OR data->'execution'->>'task_id' IS NULL OR data->'execution'->>'task_id'=$3)) OR ($3::text IS NOT NULL AND data->>'resource'=$3 AND data->'execution'->>'task_id'=$3))), recent AS (SELECT * FROM selected ORDER BY at DESC,id DESC LIMIT 200) SELECT jsonb_build_object('items',coalesce((SELECT jsonb_agg(data) FROM recent),'[]'::jsonb),'total',(SELECT count(*) FROM selected),'limit',200,'available',true)")
            .bind(project).bind(candidate).bind(task).fetch_one(&self.pool).await.map_err(unavailable)?;
        Ok(value)
    }
}
