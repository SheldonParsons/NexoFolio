use super::*;
fn fixture() -> (CandidateDiagnostics, CurationTask) {
    let now = chrono::Utc::now();
    let candidate = uuid::Uuid::new_v4();
    let project = ProjectId::new();
    let task: CurationTask = serde_json::from_value(json!({"id":uuid::Uuid::new_v4(),"project_id":project,"target":null,"parent":null,"candidate_id":candidate,"actor":{"kind":"unknown"},"state":"waiting","generation":7,"run_id":uuid::Uuid::new_v4(),"lease_until":null,"created_at":now,"updated_at":now,"last_started_at":now,"message":"password=PRIVATE_BODY","next_retry_at":now+chrono::TimeDelta::seconds(90),"retry_reason":"model_unavailable","retry_count":7,"timing":{"since":now,"queued_ms":10,"running_ms":20,"waiting_ms":30}})).unwrap();
    let candidate = serde_json::from_value(json!({"summary":{"id":candidate,"project_id":project,"target":null,"state":"partial","created_at":now,"updated_at":now,"commands":3,"progress":{"task_state":"running","run_id":uuid::Uuid::new_v4(),"status":"failed","stage":"fields","message":"sk-secret","heartbeat_at":now-chrono::TimeDelta::hours(1)}},"batches":[],"operations":[]})).unwrap();
    (candidate, task)
}
#[test]
fn authoritative_task_overrides_stale_checkpoint_and_preserves_safe_execution_identity() {
    let (mut c, mut task) = fixture();
    for (state, expected) in [
        (TaskState::Waiting, "waiting"),
        (TaskState::NeedsAttention, "needs_attention"),
        (TaskState::Stopped, "stopped"),
        (TaskState::Done, "review"),
    ] {
        task.state = state;
        if state != TaskState::Waiting {
            task.next_retry_at = None;
        }
        let events = json!({"items":[{"action":"curation.suspended","execution":{"task_id":task.id,"run_id":task.run_id,"generation":7},"message":"SECRET_PROMPT"}],"available":true,"total":1});
        let p = package(Some(&c), Some(&task), task.project_id, "synthetic", events);
        assert_eq!(p["candidate"]["status"], expected);
        assert_eq!(p["candidate"]["task_state"], json!(state));
        assert_eq!(p["task"]["retry_count"], 7);
        assert_eq!(p["task"]["next_retry_at"], json!(task.next_retry_at));
        assert_eq!(p["candidate"]["stage"], Value::Null);
        assert_eq!(p["liveness"]["heartbeat_stale"], false);
        assert_eq!(
            p["related_events"]["items"][0]["execution"]["task_id"],
            json!(task.id)
        );
        assert_eq!(p["task"]["timing"]["running_ms"], 20);
        for secret in ["PRIVATE_BODY", "sk-secret", "SECRET_PROMPT"] {
            assert!(!p.to_string().contains(secret));
        }
    }
    c.summary.state = CandidateState::Applied;
    assert_eq!(summary(&c.summary, Some(&task))["status"], "applied");
}
#[test]
fn task_without_candidate_and_legacy_candidate_do_not_fabricate_current_schedule() {
    let (c, mut t) = fixture();
    t.candidate_id = None;
    t.state = TaskState::Queued;
    t.next_retry_at = None;
    t.timing.since = None;
    let p = package(None, Some(&t), t.project_id, "synthetic", json!({}));
    assert!(p["candidate"].is_null());
    assert_eq!(p["subject"]["kind"], "task");
    assert_eq!(p["subject"]["status"], "queued");
    assert!(p["counts"]["total"].is_null());
    assert!(p["task"]["timing"].is_null());
    let legacy = package(Some(&c), None, c.summary.project_id, "synthetic", json!({}));
    assert!(legacy["task"].is_null());
    assert!(legacy["candidate"]["stage"].is_null());
    assert!(legacy["candidate"]["run_id"].is_null());
    assert_eq!(
        legacy["candidate"]["checkpoint_run_id"],
        c.summary.progress["run_id"]
    );
    assert_eq!(legacy["candidate"]["checkpoint_stage"], "fields");
    assert!(legacy["candidate"]["task_state"].is_null());
    assert!(legacy["candidate"]["next_retry_at"].is_null());
    assert_eq!(legacy["coverage"]["task_lookup"], "not_recorded");
    let mut running_legacy = c.clone();
    running_legacy.summary.state = CandidateState::Running;
    let old_run = package(
        Some(&running_legacy),
        None,
        c.summary.project_id,
        "synthetic",
        json!({}),
    );
    assert_eq!(old_run["candidate"]["status"], "unknown");
    assert!(old_run["liveness"]["heartbeat_age_seconds"].is_null());
    assert_eq!(old_run["liveness"]["heartbeat_stale"], false);
    t.candidate_id = Some(uuid::Uuid::new_v4());
    assert!(
        package(
            Some(&c),
            Some(&t),
            c.summary.project_id,
            "synthetic",
            json!({})
        )["task"]
            .is_null()
    );
}
