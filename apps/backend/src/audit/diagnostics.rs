//! Strict allowlist export from persisted candidate metadata. No model inputs or free-form errors.
use nexofolio_contracts::knowledge::{CandidateDiagnostics, CandidateState, CandidateSummary};
use serde_json::{Value, json};

fn token(value: &Value) -> Value {
    value
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 100
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-.:/".contains(c))
                && !s.starts_with("sk-")
                && !s.to_ascii_lowercase().contains("token")
                && !s.to_ascii_lowercase().contains("password")
        })
        .map(|s| json!(s))
        .unwrap_or(Value::Null)
}
fn time(value: &Value) -> Value {
    value
        .as_str()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|v| json!(v))
        .unwrap_or(Value::Null)
}
fn id(value: &Value) -> Value {
    value
        .as_str()
        .and_then(|v| uuid::Uuid::parse_str(v).ok())
        .map(|v| json!(v))
        .unwrap_or(Value::Null)
}
fn fields(value: &Value, keys: &[&str]) -> Value {
    let mut out = json!({});
    for key in keys {
        out[*key] = value[*key].as_u64().map_or(Value::Null, |v| json!(v));
    }
    out
}
fn bounded_rows(value: &Value, limit: usize, map: impl Fn(&Value) -> Value) -> Value {
    let rows = value.as_array();
    let total = rows.map_or(0, Vec::len);
    json!({"items":rows.into_iter().flatten().take(limit).map(map).collect::<Vec<_>>(),"recorded":total,"included":total.min(limit),"truncated":total>limit})
}
pub(super) fn status(c: &CandidateSummary) -> &'static str {
    let p = &c.progress;
    if c.state == CandidateState::Applied {
        return "applied";
    }
    if c.state == CandidateState::Discarded {
        return "discarded";
    }
    if c.state == CandidateState::Running {
        return "running";
    }
    if p["attempt_history"]
        .as_array()
        .and_then(|v| v.last())
        .is_some_and(|a| a["outcome"] == "cancelled")
    {
        return "stopped";
    }
    match c.state {
        CandidateState::Ready => "review",
        CandidateState::Partial if p["status"] == "failed" && p["failure_code"].is_string() => {
            "failed"
        }
        CandidateState::Partial => "gaps",
        CandidateState::Interrupted if p["status"] == "failed" => "failed",
        _ => "interrupted",
    }
}
pub(super) fn summary(c: &CandidateSummary) -> Value {
    let p = &c.progress;
    json!({"id":c.id,"project_id":c.project_id,"target":c.target,"state":c.state,"status":status(c),"created_at":c.created_at,"updated_at":c.updated_at,"stage":token(&p["stage"]),"failure_code":token(&p["failure_code"]),"run_id":id(&p["run_id"]),"completed":p["completed"].as_u64(),"total":p["total"].as_u64(),"fields_failed":p["fields_failed"].as_u64(),"issue_count":p["issues"].as_array().map(Vec::len)})
}
pub(super) fn package(c: &CandidateDiagnostics, project: &str, events: Value) -> Value {
    let p = &c.summary.progress;
    let attempts = bounded_rows(
        &p["attempt_history"],
        200,
        |v| json!({"run_id":id(&v["run_id"]),"started_at":time(&v["started_at"]),"ended_at":time(&v["ended_at"]),"outcome":token(&v["outcome"])}),
    );
    let activities = bounded_rows(
        &p["activity"],
        200,
        |v| json!({"id":v["id"].as_u64(),"at":time(&v["at"]),"stage":token(&v["stage"]),"kind":token(&v["kind"]),"endpoint":id(&v["endpoint"]),"fields":v["fields"].as_u64(),"batch":v["batch"].as_u64(),"elapsed_ms":v["elapsed_ms"].as_u64()}),
    );
    let issues = bounded_rows(
        &p["issues"],
        1000,
        |v| json!({"endpoint":id(&v["endpoint"]),"stage":token(&v["stage"]),"code":token(&v["code"]),"fields":v["fields"].as_u64(),"retryable":v["retryable"].as_bool()}),
    );
    let batches = bounded_rows(
        &json!(c.batches),
        500,
        |v| json!({"id":id(&v["id"]),"state":token(&v["state"]),"stage":token(&v["author"]["pass"]),"model":token(&v["author"]["model"]),"commands":v["commands"].as_u64()}),
    );
    let operations = bounded_rows(
        &json!(c.operations),
        200,
        |v| json!({"id":id(&v["id"]),"action":token(&v["action"]),"at":time(&v["at"]),"actor_kind":token(&v["actor"]["kind"]),"actor_id":id(&v["actor"]["id"]),"source":token(&v["source"])}),
    );
    let mut recent_events = bounded_rows(
        &events["items"],
        200,
        |v| json!({"at":time(&v["at"]),"kind":token(&v["kind"]),"action":token(&v["action"]),"code":token(&v["code"]),"request_id":id(&v["request_id"]),"status":v["status"].as_u64(),"duration_ms":v["duration_ms"].as_u64()}),
    );
    if let Some(total) = events["total"].as_u64() {
        recent_events["recorded"] = json!(total);
        recent_events["truncated"] = json!(total > recent_events["included"].as_u64().unwrap_or(0));
    }
    let heartbeat_age = p["heartbeat_at"]
        .as_str()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|v| {
            (chrono::Utc::now() - v.with_timezone(&chrono::Utc))
                .num_seconds()
                .max(0)
        });
    json!({"diagnostic_service_version":env!("CARGO_PKG_VERSION"),"liveness":{"heartbeat_age_seconds":heartbeat_age,"heartbeat_stale":c.summary.state==CandidateState::Running && heartbeat_age.is_some_and(|v|v>120),"threshold_seconds":120},"schema_version":1,"generated_at":chrono::Utc::now(),"project":{"id":c.summary.project_id,"name":project},"candidate":summary(&c.summary),
        "timing":{"started_at":time(&p["started_at"]),"attempt_started_at":time(&p["attempt_started_at"]),"last_progress_at":time(&p["updated_at"]),"heartbeat_at":time(&p["heartbeat_at"])},
        "counts":fields(p,&["completed","total","generated","unchanged","fields_completed","fields_total","fields_failed","fields_pending","fields_reused","fields_in_scope","fields_updated","retries","model_calls","model_ms","save_ms","attempts","recovery_sweeps","total_batches","completed_batches"]),
        "preparation":fields(&p["preparation"],&["endpoints_read","samples_read","samples_unavailable","fields","values"]),
        "attempts":attempts,"activities":activities,"issues":issues,"batches":batches,"operations":operations,"related_events":recent_events,
        "coverage":{"source":"persisted_candidate","activity_history":"runtime retains the latest 80 activity records; records do not carry per-event run IDs","event_total":events["total"],"event_included":events["items"].as_array().map(Vec::len),"events_available":events["available"],"unknown_fields":["provider_request_id","server_build_revision","per-activity run_id","stage end times not explicitly recorded"],"redacted_fields":["free-form messages","business paths and field values","sample bodies","execution_input","prompts","credentials"],"note":"Null means not recorded. Attempt intervals are evidence; do not infer a run ID for an event outside those intervals. Events are limited to this project and candidate; retention and collection loss can remove earlier evidence."},
        "agent_prompt":"请根据本包排查这一项目、这一轮整理。先说明真实状态，区分人工停止、待审阅和故障；按执行尝试与阶段分析错误码、重试、资料缺口和心跳。只用已记录证据，不把 HTTP 成功当作异步任务成功；不得混入其他轮次。字段为 null 表示未记录，请明确需要补充的证据。未经用户确认不要重跑、应用或清空内容。"})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allowlist_removes_free_text_and_secrets() {
        let mut c:CandidateDiagnostics=serde_json::from_value(json!({"summary":{"id":uuid::Uuid::new_v4(),"project_id":uuid::Uuid::new_v4(),"target":null,"state":"interrupted","created_at":chrono::Utc::now(),"updated_at":chrono::Utc::now(),"commands":0,"progress":{"status":"failed","message":"sk-secret","api_key":"secret-value","activity":[{"kind":"retry","stage":"fields","message":"password=secret"}],"issues":[{"code":"RATE_LIMITED","message":"body secret"}],"attempt_history":[{"outcome":"cancelled"}]}},"batches":[],"operations":[]})).unwrap();
        assert_eq!(status(&c.summary), "stopped");
        let text = package(
            &c,
            "fixture",
            json!({"items":[],"total":0,"available":true}),
        )
        .to_string();
        for secret in [
            "sk-secret",
            "secret-value",
            "password=secret",
            "body secret",
        ] {
            assert!(!text.contains(secret));
        }
        assert!(text.contains("RATE_LIMITED"));
        c.summary.progress["issues"] = json!(vec![
            json!({"code":"RATE_LIMITED","message":"sk-secret"});
            1001
        ]);
        let capped = package(
            &c,
            "fixture",
            json!({"items":[],"total":201,"available":true}),
        );
        assert_eq!(capped["issues"]["included"], 1000);
        assert_eq!(capped["issues"]["recorded"], 1001);
        assert_eq!(capped["issues"]["truncated"], true);
        assert_eq!(capped["related_events"]["recorded"], 201);
        assert_eq!(capped["related_events"]["truncated"], true);
    }
}
