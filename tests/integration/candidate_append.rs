//! Local append: real SQL ordering, typed serde and transaction rollback.
use super::*;
use nexofolio_common::RoundId;
use nexofolio_knowledge::CandidateSession;

fn note(endpoint: EndpointId, text: &str) -> Command {
    // Omitted values/context must retain the existing serde defaults.
    serde_json::from_value(
        json!({"do":"put_field_note","endpoint":endpoint,"path":"legacy","text":text}),
    )
    .unwrap()
}
async fn content(pool: &sqlx::PgPool, id: uuid::Uuid) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('view',view,'batches',batches,'base',base,'input',execution_input) FROM knowledge.candidates WHERE id=$1")
        .bind(id).fetch_one(pool).await.unwrap()
}
#[tokio::test]
#[ignore = "private PostgreSQL only"]
async fn append_orders_fences_and_rolls_back_atomically() {
    let db = fixture::database_url();
    let store =
        PostgresKnowledge::new(&Secret::new(db.clone()), 8, Duration::from_secs(5)).unwrap();
    store.migrate().await.unwrap();
    let facts = facts();
    let p = facts.rows[0].project_id;
    let e = facts.rows[0].id;
    let q = facts.rows[2].project_id;
    let k = Arc::new(Knowledge::new(store.clone(), facts));
    k.submit_task(p, None, None, None, actor()).await.unwrap();
    let task = k.claim_task(1).await.unwrap().unwrap();
    let c = k.prepare_task(&task).await.unwrap();
    let session = CandidateSession::new(&c, k.clone());
    let first = session.begin(p, author()).await.unwrap();
    let second = session.begin(p, author()).await.unwrap();
    let pool = sqlx::PgPool::connect(&db).await.unwrap();
    // A typed JSON number in the existing view must survive the new direct decoder/encoder.
    sqlx::query("UPDATE knowledge.candidates SET view=jsonb_set(view,'{lifecycle}', '{\"version\":9007199254740993,\"reconciled_version\":9007199254740993,\"decisions\":[]}'::jsonb) WHERE id=$1")
        .bind(c.id).execute(&pool).await.unwrap();
    // Legacy Option defaults and unknown author properties remain valid under serde.
    sqlx::query("UPDATE knowledge.candidates SET batches=jsonb_set(batches,'{0,author}',((batches->0->'author')-'model') || '{\"future\":true}'::jsonb) WHERE id=$1")
        .bind(c.id).execute(&pool).await.unwrap();
    // UUID identity also keeps serde's accepted spelling, rather than string equality.
    sqlx::query("UPDATE knowledge.candidates SET batches=jsonb_set(batches,'{0,id}',to_jsonb('urn:uuid:' || upper(batches->0->>'id'))) WHERE id=$1")
        .bind(c.id).execute(&pool).await.unwrap();
    let before = content(&pool, c.id).await;
    session.apply(second, &[note(e, "second")]).await.unwrap();
    session
        .apply(first, &[note(e, "one"), note(e, "two")])
        .await
        .unwrap();
    // Retrying the same commands retains the established append semantics (no deduplication).
    session.apply(first, &[note(e, "two")]).await.unwrap();
    let saved = content(&pool, c.id).await;
    assert_eq!(
        serde_json::from_value::<RoundId>(saved["batches"][0]["id"].clone()).unwrap(),
        first
    );
    assert_eq!(
        saved["batches"][0]["author"],
        before["batches"][0]["author"]
    );
    assert_eq!(saved["batches"][1]["id"], json!(second));
    assert_eq!(
        saved["batches"][0]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["one", "two", "two"]
    );
    assert_eq!(saved["batches"][1]["commands"].as_array().unwrap().len(), 1);
    assert_eq!(saved["view"]["lifecycle"], before["view"]["lifecycle"]);
    assert_eq!(saved["base"], before["base"]);
    assert_eq!(session.endpoint(e).await.unwrap().fields[0].text, "two");
    // Invalid commands after a valid command cannot publish a partial projection.
    assert_eq!(
        session
            .apply(
                first,
                &[note(e, "temporary"), note(EndpointId::new(), "foreign")]
            )
            .await
            .unwrap_err(),
        KnowledgeError::WrongProject
    );
    for result in [
        session.apply(RoundId::new(), &[note(e, "bad round")]).await,
        k.append_candidate_view(q, c.id, task.run_id, first, &[note(e, "foreign")])
            .await
            .map(|_| ()),
        k.append_candidate_view(p, c.id, Some(uuid::Uuid::new_v4()), first, &[])
            .await
            .map(|_| ()),
    ] {
        assert!(result.is_err());
    }
    assert_eq!(content(&pool, c.id).await, saved);
    // Guard stale positions and lengths at the storage boundary even under the lock.
    for changed in [0, 1, 2, 3] {
        let mut tx = store.begin().await.unwrap();
        tx.lock_project(p).await.unwrap();
        let mut context = tx.candidate_append(p, c.id, first).await.unwrap().unwrap();
        let mut round = context.rounds.remove(0);
        match changed {
            0 => round.position = 1,
            1 => round.command_count += 1,
            2 => round.stored_id = Some(RoundId::new().to_string()),
            _ => round.stored_author = Some("wrong stored author".into()),
        }
        assert!(
            tx.append_candidate_commands(&context, &round, &[note(e, "wrong")], chrono::Utc::now())
                .await
                .is_err()
        );
    }
    assert_eq!(content(&pool, c.id).await, saved);
    // Force commit failure AFTER the UPDATE has run; session and both columns roll back.
    sqlx::raw_sql("CREATE FUNCTION knowledge.reject_append_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic commit failure'; END $$; CREATE CONSTRAINT TRIGGER reject_append_commit AFTER UPDATE ON knowledge.candidates DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION knowledge.reject_append_commit();")
        .execute(&pool).await.unwrap();
    assert!(
        session
            .apply(first, &[note(e, "must roll back")])
            .await
            .is_err()
    );
    assert_eq!(content(&pool, c.id).await, saved);
    assert_eq!(session.endpoint(e).await.unwrap().fields[0].text, "two");
    sqlx::raw_sql("DROP TRIGGER reject_append_commit ON knowledge.candidates; DROP FUNCTION knowledge.reject_append_commit();").execute(&pool).await.unwrap();
    // Concurrent sessions must preserve every whole batch in one of the valid serial orders.
    let mut jobs = tokio::task::JoinSet::new();
    for n in 0..8 {
        let k = k.clone();
        jobs.spawn(async move {
            k.append_candidate_view(
                p,
                c.id,
                task.run_id,
                first,
                &[note(e, &format!("{n}-a")), note(e, &format!("{n}-b"))],
            )
            .await
            .unwrap();
        });
    }
    while let Some(result) = jobs.join_next().await {
        result.unwrap();
    }
    let concurrent = content(&pool, c.id).await;
    let commands = concurrent["batches"][0]["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 19);
    let mut seen = std::collections::HashSet::new();
    for pair in commands[3..].chunks_exact(2) {
        let a = pair[0]["text"]
            .as_str()
            .unwrap()
            .strip_suffix("-a")
            .unwrap();
        assert_eq!(pair[1]["text"], format!("{a}-b"));
        assert!(seen.insert(a));
    }
    session.finish(first, RoundState::Done, None).await.unwrap();
    let closed = content(&pool, c.id).await;
    assert_eq!(
        session
            .apply(first, &[note(e, "closed")])
            .await
            .unwrap_err(),
        KnowledgeError::RoundClosed
    );
    assert_eq!(content(&pool, c.id).await, closed);
    // Duplicate round IDs are corrupt, not a licence to append to an arbitrary match.
    sqlx::query("UPDATE knowledge.candidates SET batches=batches || jsonb_build_array(jsonb_set(batches->1,'{id}',to_jsonb('urn:uuid:' || upper(batches->1->>'id')))) WHERE id=$1").bind(c.id).execute(&pool).await.unwrap();
    let duplicate = content(&pool, c.id).await;
    assert_eq!(
        session.apply(second, &[]).await.unwrap_err(),
        KnowledgeError::RoundClosed
    );
    assert_eq!(content(&pool, c.id).await, duplicate);
    sqlx::query("UPDATE knowledge.candidates SET batches=batches-2 WHERE id=$1")
        .bind(c.id)
        .execute(&pool)
        .await
        .unwrap();
    expire(&store, &k.task(p, task.id).await.unwrap()).await;
    let expired = content(&pool, c.id).await;
    assert!(session.apply(second, &[note(e, "expired")]).await.is_err());
    assert_eq!(content(&pool, c.id).await, expired);
    k.stop_task(p, task.id).await.unwrap();
    let stopped = content(&pool, c.id).await;
    assert!(session.apply(second, &[note(e, "stopped")]).await.is_err());
    assert_eq!(content(&pool, c.id).await, stopped);
    pool.close().await;
    store.close().await;
}
