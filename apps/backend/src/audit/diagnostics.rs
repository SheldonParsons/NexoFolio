//! Strict allowlist export from persisted candidate metadata. No model inputs or free-form errors.
use super::diagnostic_task;
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::{
    CandidateDiagnostics, CandidateState, CandidateSummary, CurationTask, TaskState,
};
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
pub(super) fn status(c: &CandidateSummary, task: Option<&CurationTask>) -> &'static str {
    let p = &c.progress;
    if c.state == CandidateState::Applied {
        return "applied";
    }
    if c.state == CandidateState::Discarded {
        return "discarded";
    }
    if let Some(task) = task {
        return diagnostic_task::status(task.state);
    }
    if c.state == CandidateState::Running {
        return "unknown";
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
pub(super) fn summary(c: &CandidateSummary, task: Option<&CurationTask>) -> Value {
    let p = &c.progress;
    let current = task.is_some_and(|t| {
        t.state == TaskState::Running && t.run_id.is_some() && json!(t.run_id) == p["run_id"]
    });
    json!({"kind":"candidate","id":c.id,"project_id":c.project_id,"target":c.target,"state":c.state,"status":status(c,task),"created_at":c.created_at,"updated_at":c.updated_at,
        "commands":c.commands,"history_available":p["history_available"].as_bool().unwrap_or(true),
        "retained_commands":p["retained_commands"].as_u64(),"retained_batches":p["retained_batches"].as_u64(),
        "task_id":task.map(|t|t.id),"task_state":task.map(|t|t.state),"next_retry_at":task.and_then(|t|t.next_retry_at),
        "stage":if current { token(&p["stage"]) } else { Value::Null },"failure_code":token(&p["failure_code"]),
        "run_id":task.and_then(|t|t.run_id),"checkpoint_run_id":id(&p["run_id"]),"checkpoint_stage":token(&p["stage"]),
        "completed":p["completed"].as_u64(),"total":p["total"].as_u64(),"fields_failed":p["fields_failed"].as_u64(),"issue_count":p["issues"].as_array().map(Vec::len)})
}
pub(super) fn task_summary(task: &CurationTask) -> Value {
    json!({"kind":"task","id":task.id,"project_id":task.project_id,"target":task.target,"state":task.state,
        "status":diagnostic_task::status(task.state),"created_at":task.created_at,"updated_at":task.updated_at,
        "task_id":task.id,"task_state":task.state,"next_retry_at":task.next_retry_at,"run_id":task.run_id,
        "stage":null,"failure_code":null,"completed":null,"total":null,"fields_failed":null,"issue_count":null})
}
pub(super) fn package(
    c: Option<&CandidateDiagnostics>,
    task: Option<&CurationTask>,
    project_id: ProjectId,
    project: &str,
    events: Value,
) -> Value {
    let now = chrono::Utc::now();
    let task = task.filter(|t| {
        t.project_id == project_id && c.is_none_or(|c| t.candidate_id == Some(c.summary.id))
    });
    let subject = c
        .map(|c| summary(&c.summary, task))
        .or_else(|| task.map(task_summary));
    let p = c.map(|c| &c.summary.progress).unwrap_or(&Value::Null);
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
    let mut batches = bounded_rows(
        &c.map(|c| json!(c.batches)).unwrap_or(json!([])),
        500,
        |v| json!({"id":id(&v["id"]),"state":token(&v["state"]),"stage":token(&v["author"]["pass"]),"model":token(&v["author"]["model"]),"commands":v["commands"].as_u64()}),
    );
    let available = c.map(|_| p["history_available"].as_bool().unwrap_or(true));
    let expired = available == Some(false);
    batches["available"] = json!(available);
    batches["unavailable_reason"] = if expired {
        json!("HISTORY_EXPIRED")
    } else {
        Value::Null
    };
    if available != Some(true) {
        batches["recorded"] = p["retained_batches"]
            .as_u64()
            .map_or(Value::Null, |n| json!(n));
        batches["truncated"] = Value::Null;
    }
    let history = json!({"available":available,"reason_code":if expired {Some("HISTORY_EXPIRED")} else {None},
        "retained_commands":c.map(|c|c.summary.commands),
        "retained_batches":if expired {p["retained_batches"].as_u64()} else {c.map(|c|c.batches.len() as u64)},
        "unavailable":if expired {vec!["candidate_details","batch_metadata","commands","comparison","rollback"]} else {vec![]}});
    let operations = bounded_rows(
        &c.map(|c| json!(c.operations)).unwrap_or(json!([])),
        200,
        |v| json!({"id":id(&v["id"]),"action":token(&v["action"]),"at":time(&v["at"]),"actor_kind":token(&v["actor"]["kind"]),"actor_id":id(&v["actor"]["id"]),"source":token(&v["source"])}),
    );
    let mut recent_events = bounded_rows(
        &events["items"],
        200,
        |v| json!({"at":time(&v["at"]),"kind":token(&v["kind"]),"action":token(&v["action"]),"code":token(&v["code"]),"request_id":id(&v["request_id"]),"status":v["status"].as_u64(),"duration_ms":v["duration_ms"].as_u64(),"execution":{"task_id":id(&v["execution"]["task_id"]),"run_id":id(&v["execution"]["run_id"]),"generation":v["execution"]["generation"].as_u64()}}),
    );
    if let Some(total) = events["total"].as_u64() {
        recent_events["recorded"] = json!(total);
        recent_events["truncated"] = json!(total > recent_events["included"].as_u64().unwrap_or(0));
    }
    let running = task.is_some_and(|t| {
        t.state == TaskState::Running && t.run_id.is_some() && json!(t.run_id) == p["run_id"]
    });
    let heartbeat_age = p["heartbeat_at"]
        .as_str()
        .filter(|_| running)
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|v| {
            (chrono::Utc::now() - v.with_timezone(&chrono::Utc))
                .num_seconds()
                .max(0)
        });
    json!({"diagnostic_service_version":env!("CARGO_PKG_VERSION"),"liveness":{"heartbeat_age_seconds":heartbeat_age,"heartbeat_stale":running && heartbeat_age.is_some_and(|v|v>120),"threshold_seconds":120,"lease_stale":task.and_then(|t| if t.state==TaskState::Running { t.lease_until.map(|v|v<now) } else { None })},"schema_version":2,"generated_at":now,"project":{"id":project_id,"name":project},"subject":subject,"candidate":c.map(|c|summary(&c.summary, task)),"task":task.map(|t|diagnostic_task::evidence(t,now)),
        "timing":{"started_at":time(&p["started_at"]),"attempt_started_at":time(&p["attempt_started_at"]),"last_progress_at":time(&p["updated_at"]),"heartbeat_at":time(&p["heartbeat_at"])},
        "counts":fields(p,&["completed","total","generated","unchanged","fields_completed","fields_total","fields_failed","fields_pending","fields_reused","fields_in_scope","fields_updated","retries","model_calls","model_ms","save_ms","attempts","recovery_sweeps","total_batches","completed_batches"]),
        "preparation":fields(&p["preparation"],&["endpoints_read","samples_read","samples_unavailable","fields","values"]),
        "attempts":attempts,"activities":activities,"issues":issues,"batches":batches,"history":history,"operations":operations,"related_events":recent_events,
        "coverage":{"source":if task.is_some() && c.is_some(){"persisted_task_and_candidate"}else if task.is_some(){"persisted_task"}else{"persisted_candidate_only"},"task_lookup":if task.is_some(){"found"}else{"not_recorded"},"checkpoint_source":"candidate.progress is a saved checkpoint, not a current task schedule","activity_history":"runtime retains the latest 80 activity records; records do not carry per-event run IDs","event_total":events["total"],"event_included":events["items"].as_array().map(Vec::len),"events_available":events["available"],"unknown_fields":["provider_request_id","server_build_revision","per-activity run_id","stage end times not explicitly recorded"],"redacted_fields":["free-form messages","business paths and field values","sample bodies","execution_input","prompts","credentials"],"note":"Null means not recorded. Attempt intervals are evidence; do not infer a run ID for an event outside those intervals. Events are limited to this project and candidate; retention and collection loss can remove earlier evidence."},
        "agent_prompt":"请根据本包排查这一项目、这一轮整理。先检查 history.available 和 batches.available；HISTORY_EXPIRED 表示历史已精简，included=0 不等于原本没有批次，应采用 retained_commands/retained_batches；批次数为 null 表示原计数未知，不得据此推断未生成。先说明真实状态，区分排队、等待自动恢复、需要人工处理、人工停止、待审阅和故障；以 task 的持久状态和 observed_at 为准，不能把候选检查点当作当前执行状态；按执行尝试与阶段分析错误码、重试、资料缺口和心跳。只用已记录证据，不把 HTTP 成功当作异步任务成功；不得混入其他轮次。null 时间表示未记录或当前没有该重试计划，请结合 task.state 判断并说明未知证据。未经用户确认不要重跑、应用或清空内容。"})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allowlist_removes_free_text_and_secrets() {
        let mut c:CandidateDiagnostics=serde_json::from_value(json!({"summary":{"id":uuid::Uuid::new_v4(),"project_id":uuid::Uuid::new_v4(),"target":null,"state":"interrupted","created_at":chrono::Utc::now(),"updated_at":chrono::Utc::now(),"commands":0,"progress":{"status":"failed","message":"sk-secret","api_key":"secret-value","activity":[{"kind":"retry","stage":"fields","message":"password=secret"}],"issues":[{"code":"RATE_LIMITED","message":"body secret"}],"attempt_history":[{"outcome":"cancelled"}]}},"batches":[],"operations":[]})).unwrap();
        assert_eq!(status(&c.summary, None), "stopped");
        let text = package(
            Some(&c),
            None,
            c.summary.project_id,
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
            Some(&c),
            None,
            c.summary.project_id,
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

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod task_tests;
