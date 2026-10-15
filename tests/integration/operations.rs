//! Real isolated PostgreSQL, API process, and fixed local HTTP upstream. No paid models.
#[allow(dead_code)]
#[path = "operations/harness.rs"]
mod harness;
#[allow(dead_code)]
#[path = "operations/upstream.rs"]
mod upstream;
use harness::Harness;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use std::time::Duration;

#[tokio::test]
#[ignore = "isolated operations acceptance, scripts/test-operations.sh"]
async fn real_process_metrics_permissions_tasks_and_diagnostics() {
    let h = Harness::start().await;
    let mut ledger = vec![];
    for account in ["A80646", "a80646", "a80646"] {
        let token = harness::login(&h.client, &h.api, account).await;
        assert_eq!(
            h.client
                .get(format!("{}/v1/operations/summary", h.api))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
    }
    let mcp: Value = h
        .client
        .post(format!("{}/v1/auth/mcp-tokens", h.api))
        .bearer_auth(&h.token)
        .json(&json!({"name":"isolated operations credential boundary"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    for path in [
        "/v1/operations/summary",
        "/v1/operations/tasks",
        "/v1/operations/tasks/invalid/diagnostics",
    ] {
        assert_eq!(
            h.client
                .get(format!("{}{path}", h.api))
                .bearer_auth(mcp["token"].as_str().unwrap())
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    let ordinary = harness::login(&h.client, &h.api, "operator").await;
    for path in [
        "/v1/operations/summary",
        "/v1/operations/tasks?limit=invalid",
        "/v1/operations/tasks/invalid/diagnostics",
    ] {
        for token in [None, Some("forged-session"), Some(ordinary.as_str())] {
            let request = h
                .client
                .get(format!("{}{path}", h.api))
                .header("X-User-Account", "A80646");
            let request = if let Some(token) = token {
                request.bearer_auth(token)
            } else {
                request
            };
            assert_eq!(request.send().await.unwrap().status(), 404);
        }
    }
    sqlx::query("UPDATE access.users SET enabled=false WHERE lower(account)='a80646'")
        .execute(&h.sql)
        .await
        .unwrap();
    assert_eq!(
        h.client
            .get(format!("{}/v1/operations/summary", h.api))
            .bearer_auth(&h.token)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    sqlx::query("UPDATE access.users SET enabled=true WHERE lower(account)='a80646'")
        .execute(&h.sql)
        .await
        .unwrap();
    ledger.push(json!({"case":"authorization","result":"passed","cases":16}));
    assert!(
        h.get("/v1/operations/tasks").await["page"]["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    h.up.model.mode("hold_all");
    let one = h.submit(0, true).await;
    let two = h.submit(1, true).await;
    let queued = h.submit(0, false).await;
    h.wait(one, "running").await;
    h.wait(two, "running").await;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let summary = h.get("/v1/operations/summary").await;
            let m = &summary["snapshot"]["model"]["value"];
            if m["http_in_flight"] == 2 && m["waiting_for_permit"].as_u64().unwrap_or(0) > 0 {
                assert_eq!(h.up.model.active.load(Ordering::SeqCst), 2);
                assert_eq!(m["current_limit"], 2);
                assert_eq!(m["configured_limit"], 2);
                h.write("held-model.json", summary);
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(h.state(queued).await, "queued");
    let diag = h
        .get(&format!("/v1/operations/tasks/{queued}/diagnostics"))
        .await;
    assert_eq!(diag["schema_version"], 2);
    assert_eq!(diag["candidate"], Value::Null);
    assert_eq!(diag["task"]["id"], queued.to_string());
    h.write("task-only-diagnostic.json", diag);
    let page = h.get("/v1/operations/tasks?limit=1").await;
    assert_eq!(page["page"]["items"].as_array().unwrap().len(), 1);
    let cursor = &page["page"]["next"];
    let next = h
        .get(&format!(
            "/v1/operations/tasks?limit=1&before_at={}&before_id={}",
            url_encode(cursor["created_at"].as_str().unwrap()),
            cursor["id"].as_str().unwrap()
        ))
        .await;
    assert_ne!(
        page["page"]["items"][0]["id"],
        next["page"]["items"][0]["id"]
    );
    let filter = h
        .get(&format!(
            "/v1/operations/tasks?project={}&state=queued",
            h.projects[0]
        ))
        .await;
    assert_eq!(filter["page"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(filter["page"]["items"][0]["id"], queued.to_string());
    h.write(
        "pagination.json",
        json!({"first":page,"next":next,"filter":filter}),
    );
    let running = h.get("/v1/operations/tasks?state=running").await;
    assert_eq!(running["page"]["items"].as_array().unwrap().len(), 2);
    for task in running["page"]["items"].as_array().unwrap() {
        assert_eq!(task["stage"], "describe");
        assert_eq!(task["run_id"], task["checkpoint"]["run_id"]);
        assert!(task["heartbeat_at"].is_string());
    }
    for task in running["page"]["items"].as_array().unwrap() {
        let candidate: uuid::Uuid = task["candidate_id"].as_str().unwrap().parse().unwrap();
        let saved: Option<String> = sqlx::query_scalar(
            "SELECT progress->>'updated_at' FROM knowledge.candidates WHERE id=$1",
        )
        .bind(candidate)
        .fetch_one(&h.sql)
        .await
        .unwrap();
        assert_eq!(
            task["checkpoint"]["updated_at"],
            saved
                .map(|v| json!(
                    chrono::DateTime::parse_from_rfc3339(&v)
                        .unwrap()
                        .with_timezone(&chrono::Utc)
                ))
                .unwrap_or(Value::Null)
        );
    }
    h.write("live-running.json", running);
    h.stop(h.projects[0], queued).await;
    h.up.model.mode("ok");
    h.up.model.release.notify_waiters();
    h.wait(one, "done").await;
    h.wait(two, "done").await;
    ledger.push(json!({"case":"actual_http_permits_queue_completion","result":"passed","peak_upstream":h.up.model.peak.load(Ordering::SeqCst)}));
    h.up.model.mode("429");
    let throttled = h.submit(0, false).await;
    h.wait(throttled, "waiting").await;
    tokio::time::sleep(Duration::from_secs(6)).await;
    let summary = h.get("/v1/operations/summary").await;
    let model = &summary["snapshot"]["model"]["value"];
    assert_eq!(model["http_in_flight"], 0);
    assert_eq!(model["permits_in_use"], 0);
    assert!(model["cooldown_remaining_ms"].as_u64().unwrap() > 0);
    assert!(model["failures"]["model_rate_limited"].as_u64().unwrap() > 0);
    let waiting = h
        .get(&format!(
            "/v1/operations/tasks?state=waiting&project={}",
            h.projects[0]
        ))
        .await;
    assert!(waiting["page"]["items"][0]["stage"].is_null());
    assert!(waiting["page"]["items"][0]["heartbeat_at"].is_null());
    assert!(waiting["page"]["items"][0]["checkpoint"]["saved_batch_records"].is_null());
    h.write(
        "live-waiting.json",
        json!({"summary":summary,"tasks":waiting}),
    );
    h.stop(h.projects[0], throttled).await;
    ledger.push(
        json!({"case":"live_429_waiting_releases_permits_and_retains_progress","result":"passed"}),
    );
    // Historical task states and checkpoints are deliberately seeded for bounded list coverage.
    // Live waiting/attention/failure transitions are exercised by the unchanged step-02 script.
    let template = h.get("/v1/operations/tasks?limit=50").await;
    h.write("completed-tasks.json", template.clone());
    let mut ids = vec![];
    for n in 0..27 {
        let id = uuid::Uuid::new_v4();
        ids.push(id);
        let state = ["done", "failed", "stopped", "waiting", "needs_attention"][n % 5];
        let project = if matches!(state, "waiting" | "needs_attention") {
            uuid::Uuid::new_v4()
        } else {
            h.projects[0].to_string().parse().unwrap()
        };
        sqlx::query("INSERT INTO knowledge.curation_tasks(id,project_id,target,actor,state,created_at,updated_at,next_retry_at,retry_reason) VALUES($1,$2,$3,'{\"kind\":\"unknown\"}',$4,now()-($5 * interval '1 minute'),now(),CASE WHEN $4='waiting' THEN now()+interval '1 day' END,CASE WHEN $4='waiting' THEN 'model_unavailable' END)")
            .bind(id).bind(project).bind(uuid::Uuid::new_v4()).bind(state).bind(n as i32+1).execute(&h.sql).await.unwrap();
    }
    let first = h.get("/v1/operations/tasks").await;
    assert_eq!(first["page"]["items"].as_array().unwrap().len(), 20);
    tokio::time::sleep(Duration::from_secs(16)).await;
    let summary = h.get("/v1/operations/summary").await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM knowledge.curation_tasks")
        .fetch_one(&h.sql)
        .await
        .unwrap();
    assert_eq!(
        summary["snapshot"]["tasks"]["value"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["count"].as_i64().unwrap())
            .sum::<i64>(),
        count
    );
    let pools = summary["snapshot"]["pools"]["value"].as_array().unwrap();
    assert_eq!(pools.len(), 6);
    assert!(
        pools
            .iter()
            .all(|p| p["waiting"].is_null() && p["acquire_timeouts"].is_null())
    );
    assert!(
        summary["snapshot"]["resources"]["rss_bytes"]["value"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(summary["snapshot"]["resources"]["cpu_cores"]["value"].is_number());
    let diag = h
        .get(&format!("/v1/operations/tasks/{one}/diagnostics"))
        .await;
    assert_eq!(diag["task"]["id"], one.to_string());
    assert_eq!(diag["subject"]["stage"], Value::Null);
    for secret in [
        "fixture-model-key",
        "fixture-pass",
        "execution_input",
        "messages",
    ] {
        assert!(!serde_json::to_string(&first).unwrap().contains(secret));
    }
    let mut comparisons = vec![];
    for row in template["page"]["items"].as_array().unwrap() {
        if let Some(candidate) = row["candidate_id"].as_str() {
            let batch_count: i32 = sqlx::query_scalar(
                "SELECT jsonb_array_length(batches) FROM knowledge.candidates WHERE id=$1",
            )
            .bind(candidate.parse::<uuid::Uuid>().unwrap())
            .fetch_one(&h.sql)
            .await
            .unwrap();
            assert!(row["checkpoint"]["saved_batch_records"].is_null());
            comparisons.push(json!({"task_id":row["id"],"candidate_id":candidate,"database_batch_records":batch_count,"api_batch_records":row["checkpoint"]["saved_batch_records"]}));
        }
    }
    h.write("database-comparison.json",json!({"task_count_sql":count,"api_state_counts":summary["snapshot"]["tasks"]["value"],"checkpoint_batches":comparisons}));
    h.write("summary-resource-pools.json", summary);
    h.write("diagnostic.json", diag);
    h.write("ledger.json", json!(ledger));
    // Exact real intake HTTP ledger: accepted retry, real limiter, and temporary ledger outage.
    let mut batch: Value = serde_json::from_str(include_str!(
        "../../contracts/collect/v1/fixtures/batch/valid/exchange-without-context-or-response.json"
    ))
    .unwrap();
    batch["batch_id"] = json!(uuid::Uuid::new_v4());
    batch["target"] = json!({"project_id":h.projects[0],"environment":{"id":h.environments[0]}});
    let body = serde_json::to_vec(&batch).unwrap();
    let mut received = 0usize;
    let mut intake_ledger = vec![];
    for expected in [200, 200, 429] {
        let r = h
            .client
            .post(format!("{}/v1/collect/batches", h.api))
            .header("content-type", "application/json")
            .body(body.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), expected);
        received += body.len();
        intake_ledger.push(json!({"status":expected,"bytes":body.len(),"at":chrono::Utc::now()}));
    }
    sqlx::query("ALTER TABLE intake.batches RENAME TO operations_missing_batches")
        .execute(&h.sql)
        .await
        .unwrap();
    batch["platform"] = json!("operations-outage");
    batch["batch_id"] = json!(uuid::Uuid::new_v4());
    let body = serde_json::to_vec(&batch).unwrap();
    let response = h
        .client
        .post(format!("{}/v1/collect/batches", h.api))
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .unwrap();
    sqlx::query("ALTER TABLE intake.operations_missing_batches RENAME TO batches")
        .execute(&h.sql)
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    received += body.len();
    intake_ledger.push(json!({"status":503,"bytes":body.len(),"at":chrono::Utc::now()}));
    let intake = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let s = h.get("/v1/operations/summary").await;
            if s["snapshot"]["intake"]["value"]["totals"]["batches"] == 4 {
                break s;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    let totals = &intake["snapshot"]["intake"]["value"]["totals"];
    assert_eq!(totals["accepted_batches"], 2);
    assert_eq!(totals["received_bytes"], received);
    assert_eq!(totals["http_429"], 1);
    assert_eq!(totals["http_503"], 1);
    assert!(totals["fetcher_local_queue"].is_null());
    h.write(
        "intake-ledger.json",
        json!({"http":intake_ledger,"snapshot":intake}),
    );
    // Keep prior observation time on sampler failure; summary GET never waits for knowledge IO.
    let prior = h.get("/v1/operations/summary").await;
    let mut lock = h.sql.begin().await.unwrap();
    sqlx::query("LOCK TABLE knowledge.curation_tasks IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(18)).await;
    let started = std::time::Instant::now();
    let stale = h.get("/v1/operations/summary").await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        stale["snapshot"]["tasks"]["sampled_at"],
        prior["snapshot"]["tasks"]["sampled_at"]
    );
    assert_eq!(
        stale["snapshot"]["tasks"]["error"],
        "task_counts_unavailable"
    );
    assert!(
        stale["snapshot"]["resources"]["rss_bytes"]["sampled_at"]
            != prior["snapshot"]["resources"]["rss_bytes"]["sampled_at"]
    );
    h.write("failed-count-sample-keeps-progress.json", stale);
    lock.rollback().await.unwrap();
    h.close().await;
}
fn url_encode(value: &str) -> String {
    value.replace('+', "%2B")
}

#[tokio::test]
#[ignore = "browser and overhead fixture, isolated PostgreSQL"]
async fn serve_operations_fixture() {
    let h = Harness::start().await;
    let task = h.submit(0, false).await;
    h.wait(task, "done").await;
    for n in 0..27 {
        sqlx::query("INSERT INTO knowledge.curation_tasks(id,project_id,target,actor,state,created_at,updated_at) VALUES($1,$2,$3,'{\"kind\":\"unknown\"}',$4,now()-($5 * interval '1 minute'),now())")
            .bind(uuid::Uuid::new_v4()).bind(h.projects[0].to_string().parse::<uuid::Uuid>().unwrap()).bind(uuid::Uuid::new_v4()).bind(if n%2==0 {"done"} else {"stopped"}).bind(n+1).execute(&h.sql).await.unwrap();
    }
    h.write("ready.json",json!({"api":h.api,"pid":h.process.id(),"projects":h.projects,"endpoints":h.endpoints,"environments":h.environments,"task":task}));
    tokio::time::timeout(Duration::from_secs(7200), async {
        while !h.out.join("stop").exists() {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap();
    h.close().await;
}

#[path = "operations/projection.rs"]
mod projection;
