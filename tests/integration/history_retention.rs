//! Only synthetic data in the runner-owned PostgreSQL. Fixed policy time; real locks/IO.
use super::*;
use chrono::{DateTime, TimeDelta, Utc};
use nexofolio_common::RoundId;
use nexofolio_contracts::field_values::*;
use nexofolio_knowledge_adapter::RetentionReport;
use sqlx::PgPool;

fn now() -> DateTime<Utc> {
    "2026-11-01T00:00:00Z".parse().unwrap()
}
fn uid(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}
fn dbid(p: ProjectId) -> uuid::Uuid {
    p.to_string().parse().unwrap()
}
async fn setup() -> (PostgresKnowledge, PgPool, Facts) {
    let url = fixture::database_url();
    let store =
        PostgresKnowledge::new(&Secret::new(url.clone()), 8, Duration::from_secs(3)).unwrap();
    store.migrate().await.unwrap();
    let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options.options([("search_path", "knowledge")]))
        .await
        .unwrap();
    (store, pool, facts())
}
fn candidate(p: ProjectId, e: EndpointId, n: u128, days: i64) -> Candidate {
    let base = KnowledgeSnapshot {
        endpoints: vec![e],
        ..Default::default()
    };
    Candidate {
        id: uid(n),
        project_id: p,
        target: Some(e),
        parent: None,
        state: CandidateState::Discarded,
        created_at: now() - TimeDelta::days(days),
        updated_at: now() - TimeDelta::days(days),
        base_hash: "synthetic".into(),
        base: base.clone(),
        view: base,
        batches: vec![],
        progress: json!({}),
        execution_input: Some(json!({"version":2,"prepared":{"inputs":{}},"legacy":true})),
        applied_rounds: vec![],
        reviewed_by: None,
        operations: vec![],
    }
}
async fn save(s: &PostgresKnowledge, c: &Candidate) {
    let mut tx = s.begin().await.unwrap();
    tx.lock_project(c.project_id).await.unwrap();
    tx.put_candidate(c).await.unwrap();
    tx.commit().await.unwrap();
}
fn note(e: EndpointId, id: uuid::Uuid) -> FieldNote {
    FieldNote {
        endpoint: e,
        path: "response_body:200:id".into(),
        text: "保留的人工知识".into(),
        values: vec![],
        context: FieldContext {
            observations: Some(FieldObservationSummary {
                archive_id: Some(id),
                total: 1,
                hash: "values".into(),
                states: Default::default(),
            }),
            ..Default::default()
        },
        author: Author::Person {
            user: UserId::new(),
        },
        at: now(),
    }
}
fn archive(e: EndpointId) -> FieldValueArchive {
    serde_json::from_value(json!({"version":1,"documents":{"doc":{
        "source":{"endpoint":e,"example":"synthetic-source","environment":null,"location":null,"pointer":null,"page":"https://fixture.invalid/source","sequence":1,"platform":"fixture","complete":true},
        "observed_at":null,"observation_key":null,"representation":"json","state":"full","root":{"id":9007199254740993u64}}},
        "fields":{format!("{e}/response_body:200:id"):{"values":[{"id":"value-1","value_type":"number","representation":"json","occurrences":[{"document":"doc","pointer":"/id"}]}],"coverage":[],"hash":"values"}}})).unwrap()
}
async fn values(s: &PostgresKnowledge, c: &Candidate, e: EndpointId, chunked: bool) {
    values_with_padding(s, c, e, chunked, 0).await;
}
async fn values_with_padding(
    s: &PostgresKnowledge,
    c: &Candidate,
    e: EndpointId,
    chunked: bool,
    padding: usize,
) {
    let mut tx = s.begin().await.unwrap();
    tx.lock_project(c.project_id).await.unwrap();
    let mut a = archive(e);
    if padding > 0 {
        a.documents.get_mut("doc").unwrap().root["padding"] = json!("x".repeat(padding));
    }
    if chunked {
        let snapshot = uid(9000 + c.id.as_u128());
        let doc = serde_json::to_string(a.documents.get("doc").unwrap()).unwrap();
        a.documents.clear();
        let index = serde_json::to_string(&a).unwrap();
        let doc_ref = FrozenRef::of(&doc);
        let values_ref = FrozenRef::of(&index);
        let runtime = FrozenRef::of("[]");
        for (r, content) in [
            (&doc_ref, doc.as_str()),
            (&values_ref, index.as_str()),
            (&runtime, "[]"),
        ] {
            tx.put_frozen_chunk(c.project_id, c.id, snapshot, &r.sha256, content)
                .await
                .unwrap();
        }
        let m = FrozenManifest {
            version: 1,
            project: c.project_id,
            candidate: c.id,
            snapshot,
            endpoints: [(
                e,
                FrozenEndpoint {
                    facts: runtime.clone(),
                    inputs: vec![FrozenFieldBlock {
                        keys: vec![],
                        content: runtime.clone(),
                    }],
                    values: values_ref,
                    documents: [("doc".into(), doc_ref)].into(),
                },
            )]
            .into(),
            links: runtime,
        };
        let mut input = json!({"version":3,"manifest":m,"summaries":[],"baseline_batches":[],"lifecycle_version":0,"legacy":false});
        input["manifest_hash"] = json!(frozen_input_hash(&input));
        let mut c = c.clone();
        c.execution_input = Some(input);
        tx.put_candidate(&c).await.unwrap();
    } else {
        tx.put_value_archive(c.project_id, c.id, &a).await.unwrap();
    }
    tx.commit().await.unwrap();
}
async fn readable(
    k: &Knowledge<PostgresKnowledge, Facts>,
    p: ProjectId,
    e: EndpointId,
    id: uuid::Uuid,
) {
    let q = FieldValueQuery {
        project: p,
        archive: id,
        endpoint: e,
        field: "response_body:200:id".into(),
        offset: 0,
        limit: 10,
        environment: None,
    };
    let page = k.observed_values(q.clone()).await.unwrap();
    assert_eq!(page.summary.archive_id, Some(id));
    assert_eq!(page.summary.total, 1);
    assert_eq!(page.items[0].sources[0].source.example, "synthetic-source");
    assert_eq!(
        page.items[0].sources[0].source.pointer.as_deref(),
        Some("/id")
    );
    assert_eq!(
        k.observed_value(q, "value-1").await.unwrap().to_string(),
        "9007199254740993"
    );
}
async fn enabled(s: &PostgresKnowledge, p: ProjectId) {
    let r = s
        .retain_history(p, now() - TimeDelta::days(7), None, 20, true)
        .await
        .unwrap();
    assert_eq!(r.skipped["activation_grace_7_days"], 1);
}
async fn run(s: &PostgresKnowledge, p: ProjectId, at: DateTime<Utc>) -> RetentionReport {
    let r = s.retain_history(p, at, None, 20, true).await.unwrap();
    println!("{}", serde_json::to_string(&r).unwrap());
    r
}

#[tokio::test]
#[ignore = "private PostgreSQL only"]
async fn boundaries_references_and_real_knowledge_reads() {
    let (s, pool, f) = setup().await;
    let p = f.rows[0].project_id;
    let e = f.rows[0].id;
    let k = Knowledge::new(s.clone(), f.clone());
    for n in 1..=12 {
        let c = candidate(p, e, n, 60);
        save(&s, &c).await;
        values(&s, &c, e, n == 2).await;
    }
    // Five recent completions plus exact and adjacent 30-day / 7-day boundaries.
    for (n, days, seconds) in [
        (13, 30, 1),
        (14, 30, 0),
        (15, 7, 1),
        (16, 7, 0),
        (17, 7, -1),
        (18, 1, 0),
        (19, 2, 0),
    ] {
        let mut c = candidate(p, e, n, days);
        c.updated_at -= TimeDelta::seconds(seconds);
        save(&s, &c).await;
    }
    let mut tx = s.begin().await.unwrap();
    tx.put_field_note(p, &note(e, uid(2))).await.unwrap();
    tx.commit().await.unwrap();
    let mut pending = candidate(p, e, 20, 90);
    pending.state = CandidateState::Ready;
    pending.view.fields.push(note(e, uid(1)));
    save(&s, &pending).await;
    let mut frozen = candidate(p, e, 21, 90);
    frozen.state = CandidateState::Interrupted;
    frozen.execution_input = Some(
        json!({"version":2,"prepared":{"inputs":{e.to_string():[{"observations":note(e,uid(3)).context.observations}]}}}),
    );
    save(&s, &frozen).await;
    // Unpublished v3 preparation chunks can refer to older archives too.
    let mut tx = s.begin().await.unwrap();
    let text = json!([{"observations":note(e,uid(4)).context.observations}]).to_string();
    tx.put_frozen_chunk(p, frozen.id, uid(8000), &FrozenRef::of(&text).sha256, &text)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let kept = Round {
        id: RoundId::new(),
        project_id: p,
        author: author(),
        started_at: now() - TimeDelta::days(2),
        finished_at: Some(now() - TimeDelta::days(2)),
        state: RoundState::Done,
        summary: Some("保留跨轮回滚".into()),
    };
    let expired = Round {
        id: RoundId::new(),
        started_at: now() - TimeDelta::days(60),
        finished_at: Some(now() - TimeDelta::days(60)),
        ..kept.clone()
    };
    let mut tx = s.begin().await.unwrap();
    for r in [&kept, &expired] {
        tx.create_round(r).await.unwrap();
        tx.record_revision(&Revision {
            round: r.id,
            seq: 0,
            target: Target::FieldNote {
                endpoint: e,
                path: note(e, uid(5)).path,
            },
            before: Some({
                let mut v = serde_json::to_value(note(e, uid(5))).unwrap();
                v["values"] = json!([]);
                v
            }),
        })
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    sqlx::query("UPDATE candidates SET state='applied',applied_rounds=$2 WHERE id=$1")
        .bind(uid(6))
        .bind(json!([expired.id]))
        .execute(&pool)
        .await
        .unwrap();
    enabled(&s, p).await;
    assert_eq!(run(&s, p, now() - TimeDelta::seconds(1)).await.scanned, 0);
    let preview = s.retain_history(p, now(), None, 20, false).await.unwrap();
    assert!(!preview.committed);
    assert!(preview.compacted > 0);
    assert!(k.candidate(p, uid(6)).await.is_ok(), "preview rolls back");
    let r = run(&s, p, now()).await;
    assert_eq!(r.scanned, 20);
    assert!(r.compacted >= 12);
    assert!(matches!(
        k.candidate(p, uid(6)).await,
        Err(KnowledgeError::HistoryExpired)
    ));
    assert!(matches!(
        k.roll_back(expired.id).await,
        Err(KnowledgeError::HistoryExpired)
    ));
    let rows = k.candidates(p, 50).await.unwrap();
    let old = rows.iter().find(|c| c.id == uid(6)).unwrap();
    assert_eq!(old.progress["history_available"], false);
    assert!(
        old.progress["history_message"]
            .as_str()
            .unwrap()
            .contains("回滚已到期")
    );
    assert!(k.candidate(p, uid(13)).await.is_err());
    assert!(k.candidate(p, uid(14)).await.is_ok());
    assert!(
        k.candidate(p, uid(16))
            .await
            .unwrap()
            .execution_input
            .is_none()
    );
    assert!(
        k.candidate(p, uid(17))
            .await
            .unwrap()
            .execution_input
            .is_some()
    );
    for id in 1..=5 {
        readable(&k, p, e, uid(id)).await;
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM frozen_input_chunks WHERE candidate_id=$1"
        )
        .bind(uid(2))
        .fetch_one(&pool)
        .await
        .unwrap(),
        2,
        "v3 runtime removed, index and document kept"
    );
    let grace = run(&s, p, now() + TimeDelta::days(7) - TimeDelta::seconds(1)).await;
    assert_eq!(grace.archives_reclaimed, 0);
    let collected = run(&s, p, now() + TimeDelta::days(7)).await;
    assert!(collected.archives_reclaimed >= 7);
    for id in 1..=5 {
        readable(&k, p, e, uid(id)).await;
    }
    k.roll_back(kept.id).await.unwrap();
    assert_eq!(
        k.endpoint(e).await.unwrap().fields[0]
            .context
            .observations
            .as_ref()
            .unwrap()
            .archive_id,
        Some(uid(5))
    );
    assert_eq!(
        run(&s, p, now() + TimeDelta::days(7))
            .await
            .archives_reclaimed,
        0,
        "safe re-entry"
    );
    // The latest five are retained even if all five are much older than 30 days.
    let q = f.rows[2].project_id;
    let qe = f.rows[2].id;
    for n in 100..106 {
        save(&s, &candidate(q, qe, n, 60 + (106 - n) as i64)).await;
    }
    enabled(&s, q).await;
    assert_eq!(run(&s, q, now()).await.compacted, 1);
    for n in 101..106 {
        assert!(k.candidate(q, uid(n)).await.is_ok());
    }
    pool.close().await;
    s.close().await;
}

#[tokio::test]
#[ignore = "private PostgreSQL only"]
async fn protected_states_and_bounded_history() {
    let (s, pool, f) = setup().await;
    let p = f.rows[0].project_id;
    let e = f.rows[0].id;
    let k = Knowledge::new(s.clone(), f.clone());
    for (n, state) in [
        CandidateState::Running,
        CandidateState::Ready,
        CandidateState::Partial,
        CandidateState::Interrupted,
    ]
    .into_iter()
    .enumerate()
    {
        let mut c = candidate(p, e, n as u128 + 1, 90);
        c.state = state;
        save(&s, &c).await;
    }
    for (i, state) in ["queued", "running", "waiting", "needs_attention"]
        .into_iter()
        .enumerate()
    {
        let c = candidate(p, e, i as u128 + 10, 90);
        save(&s, &c).await;
        sqlx::query("INSERT INTO curation_tasks(id,project_id,target,candidate_id,actor,state,generation,created_at,updated_at,timing) VALUES($1,$2,$3,$4,'{\"kind\":\"unknown\"}',$5,0,$6,$6,'{}')")
            .bind(uid(1000+i as u128)).bind(dbid(p)).bind(uid(5000+i as u128)).bind(c.id).bind(state).bind(c.updated_at).execute(&pool).await.unwrap();
    }
    // 200 modest synthetic history rows: bounded cursor, no duplicate mutations.
    for n in 100..300 {
        let mut c = candidate(p, e, n, 90);
        for field in 0..24 {
            let mut generated = note(e, uid(n));
            generated.context.observations = None;
            generated.author = author();
            generated.path = format!("response_body:200:field{field}");
            generated.text = "synthetic field explanation ".repeat(40);
            c.base.fields.push(generated);
        }
        c.view = c.base.clone();
        save(&s, &c).await;
    }
    enabled(&s, p).await;
    let start = std::time::Instant::now();
    let first = run(&s, p, now()).await;
    assert_eq!(first.scanned, 20);
    assert_eq!(first.skipped["recoverable_or_active"], 8);
    let second = s
        .retain_history(p, now(), first.next, 20, true)
        .await
        .unwrap();
    assert_eq!(second.scanned, 20);
    assert!(second.next > first.next);
    for n in [1, 2, 3, 4, 10, 11, 12, 13] {
        assert!(
            k.candidate(p, uid(n))
                .await
                .unwrap()
                .execution_input
                .is_some()
        );
    }
    assert!(
        k.resume_candidate(p, uid(100), uid(8888), json!({}))
            .await
            .is_err(),
        "released cannot recover"
    );
    // A real stop/resume path still works after years of synthetic policy time.
    let q = f.rows[2].project_id;
    let stopped = k.create_candidate(q, None, None).await.unwrap();
    k.cancel_candidate(q, stopped.id).await.unwrap();
    let frozen = json!({"version":2,"summaries":[f.rows[2]],"prepared":{"facts":[],"inputs":{},"links":[],"authentication":{}},"baseline_batches":[],"legacy":false});
    k.resume_candidate(q, stopped.id, uid(7001), frozen.clone())
        .await
        .unwrap();
    k.cancel_candidate(q, stopped.id).await.unwrap();
    enabled(&s, q).await;
    let protected = run(&s, q, now() + TimeDelta::days(3650)).await;
    assert_eq!(protected.skipped["recoverable_or_active"], 1);
    assert_eq!(
        k.resume_candidate(q, stopped.id, uid(7002), frozen.clone())
            .await
            .unwrap()
            .execution_input,
        Some(frozen)
    );
    assert!(s.retain_history(p, now(), None, 21, true).await.is_err());
    let foreign = Round {
        id: RoundId::new(),
        project_id: q,
        author: author(),
        started_at: now() - TimeDelta::days(90),
        finished_at: Some(now() - TimeDelta::days(90)),
        state: RoundState::Done,
        summary: Some("foreign fixture".into()),
    };
    let mut tx = s.begin().await.unwrap();
    tx.create_round(&foreign).await.unwrap();
    tx.record_revision(&Revision {
        round: foreign.id,
        seq: 0,
        target: Target::FieldNote {
            endpoint: f.rows[2].id,
            path: "fixture".into(),
        },
        before: None,
    })
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let mut corrupt = candidate(p, e, 500, 100);
    corrupt.applied_rounds.push(foreign.id);
    save(&s, &corrupt).await;
    let rejected = s
        .retain_history(p, now(), Some(uid(499)), 1, true)
        .await
        .unwrap();
    assert_eq!(rejected.skipped["retained_rollback_group"], 1);
    assert_eq!(rejected.compacted, 0);
    let mut tx = s.begin_read().await.unwrap();
    assert_eq!(tx.revisions(foreign.id).await.unwrap().len(), 1);
    drop(tx);
    let mut unknown = candidate(p, e, 400, 90);
    unknown.execution_input = Some(json!({"version":99}));
    save(&s, &unknown).await;
    assert_eq!(run(&s, p, now()).await.skipped["unknown_input_format"], 1);
    sqlx::query("UPDATE candidates SET execution_input=NULL,base='{}' WHERE id=$1")
        .bind(unknown.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(run(&s, p, now()).await.skipped["unknown_history_shape"], 1);
    println!(
        "bounded_history_rows=208 bounded_scenario_ms={}",
        start.elapsed().as_millis()
    );
    pool.close().await;
    s.close().await;
}

#[tokio::test]
#[ignore = "private PostgreSQL only"]
async fn real_transactions_fence_references_and_rollback_interruption() {
    let (s, pool, f) = setup().await;
    let p = f.rows[0].project_id;
    let e = f.rows[0].id;
    let k = Arc::new(Knowledge::new(s.clone(), f.clone()));
    for n in 1..=8 {
        let c = candidate(p, e, n, 60);
        save(&s, &c).await;
        values(&s, &c, e, false).await;
    }
    enabled(&s, p).await;
    run(&s, p, now()).await;
    // Writer wins: hold a genuine transaction after adding a reference.
    let mut writer = s.begin().await.unwrap();
    writer.lock_project(p).await.unwrap();
    writer.put_field_note(p, &note(e, uid(1))).await.unwrap();
    let busy = run(&s, p, now() + TimeDelta::days(8)).await;
    assert_eq!(busy.skipped["project_busy"], 1);
    writer.commit().await.unwrap();
    assert_eq!(
        run(&s, p, now() + TimeDelta::days(8))
            .await
            .archives_reclaimed,
        2
    );
    readable(&k, p, e, uid(1)).await;
    let mut writer = s.begin().await.unwrap();
    writer.lock_project(p).await.unwrap();
    writer
        .remove_field_note(e, &note(e, uid(1)).path)
        .await
        .unwrap();
    writer.commit().await.unwrap();
    let fresh = run(&s, p, now() + TimeDelta::days(8)).await;
    assert_eq!(
        fresh.grace_started, 1,
        "transient reference restarts full grace"
    );
    // Cleaner wins: force its DELETE to wait on a real table lock, then start a
    // real application write. Releasing the gate lets cleanup commit first.
    let mut gate = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE field_value_archives IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *gate)
        .await
        .unwrap();
    let cleaner = {
        let s = s.clone();
        tokio::spawn(async move {
            s.retain_history(p, now() + TimeDelta::days(15), None, 20, true)
                .await
        })
    };
    wait_blocked(&pool, "DELETE FROM field_value_archives").await;
    let late = {
        let s = s.clone();
        tokio::spawn(async move {
            let mut tx = s.begin().await.unwrap();
            tx.lock_project(p).await.unwrap();
            let result = tx.put_field_note(p, &note(e, uid(1))).await;
            if result.is_ok() {
                tx.commit().await.unwrap();
            }
            result
        })
    };
    wait_blocked(&pool, "pg_advisory_xact_lock").await;
    gate.commit().await.unwrap();
    assert_eq!(cleaner.await.unwrap().unwrap().archives_reclaimed, 1);
    assert!(
        late.await.unwrap().is_err(),
        "tombstone rejects late reference"
    );
    // A real recovery transaction also waits on exactly the cleanup project lock.
    let mut lock = s.begin().await.unwrap();
    lock.lock_project(p).await.unwrap();
    let apply = {
        let k = k.clone();
        tokio::spawn(async move { k.apply_candidate(p, uid(1), UserId::new()).await })
    };
    wait_blocked(&pool, "pg_advisory_xact_lock").await;
    lock.commit().await.unwrap();
    assert_eq!(
        apply.await.unwrap().unwrap_err(),
        KnowledgeError::HistoryExpired
    );
    // Cancellation after history UPDATEs but before deleting evidence must undo
    // the whole batch, including counters and grace timestamps.
    let c = candidate(p, e, 20, 100);
    save(&s, &c).await;
    values(&s, &c, e, false).await;
    let initial = s
        .retain_history(p, now(), Some(uid(19)), 1, true)
        .await
        .unwrap();
    assert_eq!(initial.compacted, 1);
    let before: Value = sqlx::query_scalar("SELECT to_jsonb(c) FROM candidates c WHERE id=$1")
        .bind(c.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut gate = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM candidates WHERE id=$1 FOR UPDATE")
        .bind(c.id)
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    let task = {
        let s = s.clone();
        tokio::spawn(async move {
            s.retain_history(p, now() + TimeDelta::days(16), Some(uid(19)), 1, true)
                .await
        })
    };
    wait_blocked(&pool, "archive_reclaimed_at=$3").await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    gate.commit().await.unwrap();
    let mut fence = s.begin().await.unwrap();
    fence.lock_project(p).await.unwrap();
    fence.commit().await.unwrap();
    let after: Value = sqlx::query_scalar("SELECT to_jsonb(c) FROM candidates c WHERE id=$1")
        .bind(c.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        before, after,
        "aborted SQL transaction rolls back all earlier mutations"
    );
    readable(&k, p, e, c.id).await;
    let retry = s
        .retain_history(p, now() + TimeDelta::days(16), Some(uid(19)), 1, true)
        .await
        .unwrap();
    assert_eq!(retry.archives_reclaimed, 1);
    assert_eq!(retry.compacted, 0);
    assert_eq!(
        s.retain_history(p, now() + TimeDelta::days(16), Some(uid(19)), 1, true)
            .await
            .unwrap()
            .archives_reclaimed,
        0
    );
    let c = candidate(p, e, 30, 100);
    save(&s, &c).await;
    values(&s, &c, e, false).await;
    s.retain_history(p, now(), Some(uid(29)), 1, true)
        .await
        .unwrap();
    let mut gate = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM candidates WHERE id=$1 FOR UPDATE")
        .bind(c.id)
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    assert!(
        s.retain_history(p, now() + TimeDelta::days(16), Some(uid(29)), 1, true)
            .await
            .is_err()
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "database statement timeout bounds a blocked batch"
    );
    gate.commit().await.unwrap();
    readable(&k, p, e, c.id).await;
    assert_eq!(
        s.retain_history(p, now() + TimeDelta::days(16), Some(uid(29)), 1, true)
            .await
            .unwrap()
            .archives_reclaimed,
        1
    );
    println!("statement_timeout_ms={}", started.elapsed().as_millis());
    pool.close().await;
    s.close().await;
}
async fn wait_blocked(pool: &PgPool, needle: &str) {
    tokio::time::timeout(Duration::from_secs(1),async {
        loop {
            let blocked:Option<Value>=sqlx::query_scalar("SELECT jsonb_build_object('pid',pid,'wait_event',wait_event,'query',left(query,160)) FROM pg_stat_activity WHERE pid<>pg_backend_pid() AND datname=current_database() AND wait_event_type='Lock' AND position($1 in query)>0 LIMIT 1").bind(needle).fetch_optional(pool).await.unwrap();
            if let Some(blocked)=blocked {println!("observed_pg_lock={blocked}");break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("actual blocked database transaction");
}

#[path = "history_retention_limits.rs"]
mod limits;
