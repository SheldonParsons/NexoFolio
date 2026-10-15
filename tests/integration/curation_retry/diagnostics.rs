use super::*;
use nexofolio_audit_contracts::{AuditStore, Event, Execution, Limits};
use std::sync::atomic::Ordering;

async fn get(h: &Harness, path: &str) -> Value {
    h.client
        .get(format!("{}{path}", h.api.url))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap()
}
async fn package(h: &Harness, t: &CurationTask) -> Value {
    get(
        h,
        &format!("/v1/audit/curations/{}/tasks/{}", t.project_id, t.id),
    )
    .await
}
fn event(t: &CurationTask, resource: uuid::Uuid, code: &str) -> Event {
    Event {
        execution: Some(Execution {
            task_id: t.id,
            run_id: t.run_id,
            generation: t.generation,
        }),
        at: chrono::Utc::now(),
        kind: "operation".into(),
        action: "curation.suspended".into(),
        actor: "fixture".into(),
        route: "/curation".into(),
        method: "WORKER".into(),
        status: 0,
        duration_ms: 0,
        request_id: uuid::Uuid::new_v4().to_string(),
        project: t.project_id.to_string(),
        resource: resource.to_string(),
        code: code.into(),
    }
}
async fn stop(h: &Harness, t: &CurationTask) {
    assert_eq!(
        h.client
            .post(format!(
                "{}/v1/projects/{}/curate/tasks/{}/stop",
                h.api.url, t.project_id, t.id
            ))
            .bearer_auth(&h.token)
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
}
#[tokio::test]
#[ignore = "actual API process and isolated PG diagnostic export"]
async fn current_task_diagnostics_and_exact_events_survive_restarts_and_decisions() {
    let mut h = Harness::start("diagnostics").await;
    let pool = sqlx::PgPool::connect(&h.db_url).await.unwrap();
    sqlx::query("UPDATE access.users SET account='a80646' WHERE account='operator'")
        .execute(&pool)
        .await
        .unwrap();
    h.up.model.management.store(true, Ordering::SeqCst);
    let t = h.submit(0).await;
    let waiting = h.wait(&t, TaskState::Waiting, 30).await;
    let cid = waiting.candidate_id.unwrap();
    let mut legacy = event(&waiting, waiting.id, "LEGACY_MATCH");
    let good = event(&waiting, cid, "CANDIDATE_MATCH");
    let mut other = event(&waiting, uuid::Uuid::new_v4(), "OTHER_ROUND");
    other.execution.as_mut().unwrap().task_id = uuid::Uuid::new_v4();
    let mut wrong = event(&waiting, cid, "WRONG_TASK");
    wrong.execution.as_mut().unwrap().task_id = uuid::Uuid::new_v4();
    let mut foreign = event(&waiting, cid, "OTHER_PROJECT");
    foreign.project = h.projects[1].to_string();
    h.db.audit
        .write(
            &[legacy.clone(), good, other, wrong, foreign],
            &Limits::default(),
        )
        .await
        .unwrap();
    let p = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let p = package(&h, &waiting).await;
            if p["related_events"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["action"] == "curation.suspended" && e["code"] == "model_unavailable")
            {
                break p;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(p["candidate"]["status"], "waiting");
    assert_eq!(p["task"]["next_retry_at"], json!(waiting.next_retry_at));
    assert_eq!(p["task"]["run_id"], json!(waiting.run_id));
    assert_eq!(p["liveness"]["heartbeat_stale"], false);
    let text = p.to_string();
    for forbidden in [
        "OTHER_ROUND",
        "OTHER_PROJECT",
        "WRONG_TASK",
        "execution_input",
        "fixture-model-key",
    ] {
        if forbidden == "execution_input" {
            assert!(p.get(forbidden).is_none());
        } else {
            assert!(!text.contains(forbidden));
        }
    }
    assert!(text.contains("LEGACY_MATCH"));
    assert!(text.contains("CANDIDATE_MATCH"));
    assert!(
        p["related_events"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["action"] == "curation.suspended")
            .all(|e| e["execution"]["task_id"] == json!(t.id))
    );
    h.write("waiting.json", p);
    h.restart().await;
    let restored = package(&h, &waiting).await;
    assert_eq!(
        restored["task"]["next_retry_at"],
        json!(waiting.next_retry_at)
    );
    h.write("restarted.json", restored);
    h.up.model.mode("ok");
    h.wait(&t, TaskState::Done, 90).await;
    let completed = package(&h, &t).await;
    assert_eq!(completed["candidate"]["status"], "review");
    assert!(completed["task"]["next_retry_at"].is_null());
    h.write("completed.json", completed);
    h.up.model.mode("401");
    let t = h.submit(0).await;
    let attention = h.wait(&t, TaskState::NeedsAttention, 30).await;
    let p = package(&h, &t).await;
    assert_eq!(p["candidate"]["status"], "needs_attention");
    assert_eq!(p["task"]["retry_reason"], "model_rejected");
    assert!(p["task"]["next_retry_at"].is_null());
    h.write("attention.json", p);
    stop(&h, &t).await;
    let p = package(&h, &t).await;
    assert_eq!(p["candidate"]["status"], "stopped");
    h.write("stopped.json", p);
    h.up.model.mode("503");
    let t = h.submit(0).await;
    let waiting = h.wait(&t, TaskState::Waiting, 30).await;
    let response = h
        .client
        .post(format!(
            "{}/v1/projects/{}/curate/candidates/{}/apply",
            h.api.url,
            t.project_id,
            waiting.candidate_id.unwrap()
        ))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let applied = package(&h, &t).await;
    assert_eq!(applied["candidate"]["status"], "applied");
    assert_eq!(applied["task"]["state"], "stopped");
    h.write("applied.json", applied);
    h.up.model.mode("hold");
    h.restart().await;
    let holder = h.submit(0).await;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if h.up
                .model
                .calls
                .lock()
                .unwrap()
                .last()
                .is_some_and(|v| v["mode"] == "hold" && v["paths"] == json!(["f2"]))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let receipt: Value = h
        .client
        .post(format!(
            "{}/v1/projects/{}/curate",
            h.api.url, h.projects[0]
        ))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let queued = h
        .knowledge
        .task(
            h.projects[0],
            receipt["task_id"].as_str().unwrap().parse().unwrap(),
        )
        .await
        .unwrap();
    assert!(queued.candidate_id.is_none());
    let p = package(&h, &queued).await;
    assert_eq!(p["subject"]["status"], "queued");
    assert!(p["candidate"].is_null());
    assert!(p["counts"]["total"].is_null());
    h.write("queued-without-candidate.json", p);
    let running = package(&h, &holder).await;
    assert_eq!(running["task"]["state"], "running");
    h.write("running.json", running);
    stop(&h, &queued).await;
    stop(&h, &holder).await;
    h.up.model.release.notify_waiters();
    let t = h
        .knowledge
        .submit_task(h.projects[0], None, None, None, CandidateActor::Unknown)
        .await
        .unwrap();
    h.wait(&t, TaskState::NeedsAttention, 10).await;
    let p = package(&h, &t).await;
    assert!(p["candidate"].is_null());
    assert_eq!(p["subject"]["status"], "needs_attention");
    h.write("attention-without-candidate.json", p);
    stop(&h, &t).await;
    // No task row: retain legacy candidate evidence without trusting its task-looking checkpoint.
    let legacy_candidate = h
        .knowledge
        .create_candidate(h.projects[0], None, None)
        .await
        .unwrap();
    h.knowledge.checkpoint_candidate(h.projects[0],legacy_candidate.id,CandidateState::Partial,json!({"status":"running","stage":"fields","task_state":"waiting","next_retry_at":"2099-01-01T00:00:00Z","message":"password=PRIVATE_BODY"})).await.unwrap();
    legacy.resource = legacy_candidate.id.to_string();
    legacy.execution = None;
    legacy.code = "LEGACY_CANDIDATE".into();
    h.db.audit
        .write(&[legacy], &Limits::default())
        .await
        .unwrap();
    let old = get(
        &h,
        &format!(
            "/v1/audit/curations/{}/{}",
            h.projects[0], legacy_candidate.id
        ),
    )
    .await;
    assert!(old["task"].is_null());
    assert!(old["candidate"]["stage"].is_null());
    assert_eq!(old["candidate"]["checkpoint_stage"], "fields");
    assert!(old["candidate"]["task_state"].is_null());
    assert!(!old.to_string().contains("PRIVATE_BODY"));
    assert!(old.to_string().contains("LEGACY_CANDIDATE"));
    h.write("legacy.json", old);
    // Strict project scope and manager gate apply to the new task-only path too.
    assert_eq!(
        h.client
            .get(format!(
                "{}/v1/audit/curations/{}/tasks/{}",
                h.api.url, h.projects[1], attention.id
            ))
            .bearer_auth(&h.token)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    sqlx::query("UPDATE access.users SET account='operator' WHERE account='a80646'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        h.client
            .get(format!(
                "{}/v1/audit/curations/{}/tasks/{}",
                h.api.url, t.project_id, t.id
            ))
            .bearer_auth(&h.token)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    h.write("result.json",json!({"passed":true,"scope":"real API + PostgreSQL, authoritative export and exact event association","same_candidate":cid}));
    pool.close().await;
    h.close().await;
}

#[tokio::test]
#[ignore = "isolated actual Chrome fixture; command file is synthetic test control only"]
async fn serve_retry_fixes_browser_fixture() {
    let h = Harness::start("fixes-browser").await;
    h.up.model.management.store(true, Ordering::SeqCst);
    let mut last = Value::Null;
    tokio::time::timeout(Duration::from_secs(3600), async {
        while !h.out.join("stop").exists() {
            if let Ok(raw) = std::fs::read_to_string(h.out.join("command.json")) {
                let command: Value = serde_json::from_str(&raw).unwrap();
                if command["id"] != last {
                    last = command["id"].clone();
                    if command["action"] == "seed_attention" {
                        let task = h
                            .knowledge
                            .submit_task(h.projects[0], None, None, None, CandidateActor::Unknown)
                            .await
                            .unwrap();
                        let current = h.wait(&task, TaskState::NeedsAttention, 10).await;
                        assert!(current.candidate_id.is_none());
                        h.write("command-result.json", json!({"id":last,"task":current}));
                    } else if command["action"] == "release" {
                        h.up.model.release.notify_waiters();
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .unwrap();
    h.close().await;
}
