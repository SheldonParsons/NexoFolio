use super::*;
#[tokio::test]
#[ignore = "isolated PostgreSQL wait/attention/parent closure consistency"]
async fn retry_states_keep_scope_order_owner_and_parent_closure() {
    let store = PostgresKnowledge::new(
        &Secret::new(fixture::database_url()),
        8,
        Duration::from_secs(5),
    )
    .unwrap();
    store.migrate().await.unwrap();
    let pool = sqlx::PgPool::connect(&fixture::database_url())
        .await
        .unwrap();
    for temporary in [true, false] {
        for action in ["apply", "discard", "cancel"] {
            let facts = facts();
            let p = facts.rows[0].project_id;
            let q = facts.rows[2].project_id;
            let target = facts.rows[0].id;
            let k = Knowledge::new(store.clone(), facts);
            let parent = k.create_candidate(p, None, None).await.unwrap();
            k.checkpoint_candidate(p, parent.id, CandidateState::Partial, json!({}))
                .await
                .unwrap();
            let task = k
                .submit_task(p, Some(target), Some(parent.id), None, actor())
                .await
                .unwrap();
            let owner = k.claim_task(1).await.unwrap().unwrap();
            assert_eq!(owner.id, task.id);
            let c = k.prepare_task(&owner).await.unwrap();
            k.freeze_candidate_input(
                p,
                c.id,
                owner.run_id.unwrap(),
                json!({"version":2,"preserved":true}),
            )
            .await
            .unwrap();
            let suspended = k
                .suspend_task(
                    &owner,
                    TaskFailure {
                        code: if temporary {
                            "model_unavailable"
                        } else {
                            "invalid_reply"
                        }
                        .into(),
                        message: "synthetic safe reason".into(),
                        temporary,
                        retry_after_seconds: Some(90),
                    },
                    json!({"run_id":owner.run_id,"resume_available":true}),
                )
                .await
                .unwrap();
            let mut reset = pool.begin().await.unwrap();
            assert!(
                store
                    .prepare_project_reset(&mut reset, p)
                    .await
                    .unwrap()
                    .running_curations
                    > 0
            );
            reset.rollback().await.unwrap();
            assert_eq!(
                suspended.state,
                if temporary {
                    TaskState::Waiting
                } else {
                    TaskState::NeedsAttention
                }
            );
            assert!(suspended.lease_until.is_none());
            if temporary {
                assert!(
                    suspended.next_retry_at.unwrap()
                        >= suspended.updated_at + chrono::TimeDelta::seconds(90)
                );
            } else {
                assert!(suspended.next_retry_at.is_none());
            }
            assert_eq!(
                k.submit_task(p, Some(target), None, None, actor())
                    .await
                    .unwrap()
                    .id,
                task.id
            );
            let behind = k.submit_task(p, None, None, None, actor()).await.unwrap();
            assert!(k.claim_task(1).await.unwrap().is_none());
            let other = k.submit_task(q, None, None, None, actor()).await.unwrap();
            assert_eq!(k.claim_task(1).await.unwrap().unwrap().id, other.id);
            k.stop_task(q, other.id).await.unwrap();
            assert!(k.heartbeat_task(&owner).await.is_err());
            assert!(
                k.open_candidate_batch(p, c.id, owner.run_id, author())
                    .await
                    .is_err()
            );
            let before = k.candidate(p, c.id).await.unwrap();
            match action {
                "apply" => {
                    k.apply_candidate_as(p, parent.id, actor()).await.unwrap();
                }
                "discard" => k.discard_candidate_as(p, parent.id, actor()).await.unwrap(),
                _ => k.cancel_candidate(p, parent.id).await.unwrap(),
            }
            let closed = k.task(p, task.id).await.unwrap();
            assert_eq!(closed.state, TaskState::Stopped);
            assert!(closed.next_retry_at.is_none());
            let after = k.candidate(p, c.id).await.unwrap();
            assert_eq!(after.execution_input, before.execution_input);
            assert_eq!(after.view, before.view);
            assert_eq!(after.progress["task_state"], "stopped");
            let next = k.claim_task(1).await.unwrap().unwrap();
            assert_eq!(next.id, behind.id);
            k.prepare_task(&next).await.unwrap();
            k.stop_task(p, next.id).await.unwrap();
            let mut reset = pool.begin().await.unwrap();
            assert_eq!(
                store
                    .prepare_project_reset(&mut reset, p)
                    .await
                    .unwrap()
                    .running_curations,
                0
            );
            store.reset_project_content(&mut reset, p).await.unwrap();
            reset.commit().await.unwrap();
            assert!(k.task(p, task.id).await.is_err());
            assert!(k.candidate(p, c.id).await.is_err());
        }
    }
    pool.close().await;
    store.close().await;
}
#[tokio::test]
#[ignore = "fresh isolated database at first-step schema"]
async fn first_step_tasks_upgrade_without_rewriting_identity_or_history() {
    let url = fixture::database_url();
    let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_with(options.options([("search_path", "knowledge")]))
        .await
        .unwrap();
    let all = sqlx::migrate!("../../modules/knowledge/adapter/migrations");
    let mut old = sqlx::migrate::Migrator::with_migrations(
        all.iter()
            .filter(|m| m.version <= 202610120001)
            .cloned()
            .collect(),
    );
    old.create_schema("knowledge");
    old.dangerous_set_table_name("knowledge._sqlx_migrations");
    old.run(&pool).await.unwrap();
    let id = uuid::Uuid::new_v4();
    let project = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO curation_tasks(id,project_id,actor,state,generation,created_at,updated_at,message) VALUES($1,$2,'{\"kind\":\"unknown\"}','queued',0,now(),now(),'old queued identity')").bind(id).bind(project).execute(&pool).await.unwrap();
    let store = PostgresKnowledge::new(&Secret::new(url), 4, Duration::from_secs(5)).unwrap();
    store.migrate().await.unwrap();
    store.migrate().await.unwrap();
    let k = Knowledge::new(store.clone(), facts());
    let old_task = k
        .task(project.to_string().parse().unwrap(), id)
        .await
        .unwrap();
    assert_eq!(old_task.state, TaskState::Queued);
    assert_eq!(old_task.message.as_deref(), Some("old queued identity"));
    assert_eq!(old_task.retry_count, 0);
    assert!(old_task.next_retry_at.is_none());
    assert!(old_task.timing.since.is_some());
    let claimed = k.claim_task(1).await.unwrap().unwrap();
    assert_eq!(claimed.id, id);
    assert_eq!(claimed.generation, 1);
    k.stop_task(claimed.project_id, id).await.unwrap();
    store.close().await;
    pool.close().await;
}
