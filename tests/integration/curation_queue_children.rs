use super::*;

#[tokio::test]
#[ignore = "isolated TEST_DATABASE_URL required"]
async fn parent_closure_interrupts_child_and_releases_next_scope() {
    let store = PostgresKnowledge::new(
        &Secret::new(fixture::database_url()),
        8,
        Duration::from_secs(5),
    )
    .unwrap();
    store.migrate().await.unwrap();
    let mut failures = vec![];
    for action in ["apply", "discard", "cancel"] {
        let facts = facts();
        let p = facts.rows[0].project_id;
        let k = Knowledge::new(store.clone(), facts);
        k.submit_task(p, None, None, None, actor()).await.unwrap();
        let owner = k.claim_task(1).await.unwrap().unwrap();
        let parent = k.prepare_task(&owner).await.unwrap();
        k.checkpoint_candidate(
            p,
            parent.id,
            CandidateState::Partial,
            json!({"run_id":owner.run_id}),
        )
        .await
        .unwrap();
        k.submit_task(p, None, Some(parent.id), None, actor())
            .await
            .unwrap();
        let child_task = k.claim_task(1).await.unwrap().unwrap();
        let child = k.prepare_task(&child_task).await.unwrap();
        assert_eq!(child.parent, Some(parent.id));
        let batch = k
            .open_candidate_batch(p, child.id, child_task.run_id, author())
            .await
            .unwrap();
        k.append_candidate(
            p,
            child.id,
            child_task.run_id,
            batch,
            &[Command::PutEndpointNote {
                endpoint: child.base.endpoints[0],
                name: Some("saved child result".into()),
                purpose: None,
                context: Default::default(),
            }],
        )
        .await
        .unwrap();
        k.freeze_candidate_input(
            p,
            child.id,
            child_task.run_id.unwrap(),
            json!({"version":2,"fixture":"retained"}),
        )
        .await
        .unwrap();
        let before = k.candidate(p, child.id).await.unwrap();
        match action {
            "apply" => {
                k.apply_candidate_as(p, parent.id, actor()).await.unwrap();
            }
            "discard" => k.discard_candidate_as(p, parent.id, actor()).await.unwrap(),
            _ => k.cancel_candidate(p, parent.id).await.unwrap(),
        }
        assert_eq!(
            k.task(p, child_task.id).await.unwrap().state,
            TaskState::Stopped
        );
        let after = k.candidate(p, child.id).await.unwrap();
        assert_eq!(after.progress["task_state"], "stopped");
        assert_eq!(after.execution_input, before.execution_input);
        assert_eq!(after.view, before.view);
        assert_eq!(
            serde_json::to_value(&after.batches).unwrap(),
            serde_json::to_value(&before.batches).unwrap()
        );
        assert!(
            k.append_candidate(p, child.id, child_task.run_id, batch, &[])
                .await
                .is_err()
        );
        assert!(
            k.checkpoint_candidate(
                p,
                child.id,
                CandidateState::Ready,
                json!({"run_id":child_task.run_id})
            )
            .await
            .is_err()
        );
        k.submit_task(p, None, None, None, actor()).await.unwrap();
        let next = k.claim_task(1).await.unwrap().unwrap();
        let result = k.prepare_task(&next).await;
        if after.state != CandidateState::Interrupted || result.is_err() {
            failures.push(format!(
                "{action}: child={:?}, next={:?}",
                after.state,
                result.as_ref().map(|c| c.id)
            ));
        }
        k.stop_task(p, next.id).await.unwrap();
    }
    store.close().await;
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
