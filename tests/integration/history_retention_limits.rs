use super::*;

#[tokio::test]
#[ignore = "private PostgreSQL only"]
async fn incomplete_reference_proof_commits_material_progress_but_never_deletes() {
    let (s, pool, f) = setup().await;
    let p = f.rows[0].project_id;
    let e = f.rows[0].id;
    let k = Knowledge::new(s.clone(), f.clone());
    for n in 1..=8 {
        let c = candidate(p, e, n, 60);
        save(&s, &c).await;
        values_with_padding(&s, &c, e, n == 2, if n == 2 { 18 * 1024 * 1024 } else { 0 }).await;
    }
    enabled(&s, p).await;
    // Real competing transaction blocks only the reference query. Its timeout
    // must roll back the proof savepoint, not the preceding safe compaction.
    let mut gate = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE field_notes IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *gate)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let r = s.retain_history(p, now(), None, 1, true).await.unwrap();
    assert!(r.committed);
    assert_eq!(
        (
            r.compacted,
            r.inputs_released,
            r.archives_reclaimed,
            r.grace_started
        ),
        (1, 1, 0, 0)
    );
    assert_eq!(r.skipped["reference_query_incomplete"], 1);
    assert!(started.elapsed() < Duration::from_secs(2));
    println!(
        "reference_timeout_progress={}",
        serde_json::to_string(&r).unwrap()
    );
    gate.commit().await.unwrap();
    readable(&k, p, e, uid(1)).await;
    assert!(matches!(
        k.candidate(p, uid(1)).await,
        Err(KnowledgeError::HistoryExpired)
    ));
    let mut evidence = s.begin().await.unwrap();
    evidence.lock_project(p).await.unwrap();
    evidence.put_field_note(p, &note(e, uid(2))).await.unwrap();
    evidence.commit().await.unwrap();
    let large = s
        .retain_history(p, now(), Some(uid(1)), 1, true)
        .await
        .unwrap();
    assert_eq!(
        (
            large.inputs_released,
            large.chunks_deleted,
            large.archives_reclaimed
        ),
        (1, 1, 0)
    );
    assert_eq!(large.skipped["archive_referenced"], 1);
    readable(&k, p, e, uid(2)).await;
    println!(
        "large_v3_evidence_preserved={}",
        serde_json::to_string(&large).unwrap()
    );
    let mut writer = s.begin().await.unwrap();
    writer.lock_project(p).await.unwrap();
    writer.put_field_note(p, &note(e, uid(1))).await.unwrap();
    assert_eq!(
        s.retain_history(p, now() + TimeDelta::days(8), None, 1, true)
            .await
            .unwrap()
            .skipped["project_busy"],
        1
    );
    writer.commit().await.unwrap();
    assert_eq!(
        s.retain_history(p, now() + TimeDelta::days(8), None, 1, true)
            .await
            .unwrap()
            .skipped["archive_referenced"],
        1
    );
    let mut writer = s.begin().await.unwrap();
    writer.lock_project(p).await.unwrap();
    writer
        .remove_field_note(e, &note(e, uid(1)).path)
        .await
        .unwrap();
    writer.commit().await.unwrap();
    assert_eq!(
        s.retain_history(p, now() + TimeDelta::days(8), None, 1, true)
            .await
            .unwrap()
            .grace_started,
        1
    );
    // A specific over-budget frozen block is identified. It cannot authorize
    // archive deletion, yet other eligible histories still make progress.
    let mut active = candidate(p, e, 30, 90);
    active.state = CandidateState::Interrupted;
    save(&s, &active).await;
    let text=json!([{"observations":note(e,uid(1)).context.observations,"value_examples":["x".repeat(17*1024*1024)]}]).to_string();
    let mut tx = s.begin().await.unwrap();
    tx.put_frozen_chunk(p, active.id, uid(8000), &FrozenRef::of(&text).sha256, &text)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let r = s
        .retain_history(p, now() + TimeDelta::days(16), None, 3, true)
        .await
        .unwrap();
    assert_eq!(r.compacted, 1);
    assert_eq!(r.archives_reclaimed, 0);
    assert_eq!(
        r.deferred_objects[&active.id],
        "frozen_reference_block_budget"
    );
    assert_eq!(r.reference_bytes, 0);
    let grace: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT archive_unreferenced_since FROM candidates WHERE id=$1")
            .bind(uid(1))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(grace.is_none());
    println!(
        "specific_block_deferred={}",
        serde_json::to_string(&r).unwrap()
    );
    readable(&k, p, e, uid(1)).await;
    sqlx::query("DELETE FROM frozen_input_chunks WHERE candidate_id=$1")
        .bind(active.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        s.retain_history(p, now() + TimeDelta::days(16), None, 1, true)
            .await
            .unwrap()
            .grace_started,
        1
    );
    assert_eq!(
        s.retain_history(p, now() + TimeDelta::days(23), None, 1, true)
            .await
            .unwrap()
            .archives_reclaimed,
        1
    );
    let mut late = s.begin().await.unwrap();
    late.lock_project(p).await.unwrap();
    assert!(late.put_field_note(p, &note(e, uid(1))).await.is_err());
    drop(late);
    pool.close().await;
    s.close().await;
}
