use super::*;
#[tokio::test]
#[ignore = "fresh isolated nexofolio_recovery_* database required"]
async fn migration_adopts_existing_running_candidate_without_rewriting_evidence() {
    let db = fixture::database_url();
    let options: sqlx::postgres::PgConnectOptions = db.parse().unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_with(options.options([("search_path", "knowledge")]))
        .await
        .unwrap();
    let all = sqlx::migrate!("../../modules/knowledge/adapter/migrations");
    let mut old = sqlx::migrate::Migrator::with_migrations(
        all.iter()
            .filter(|m| m.version < 202610120001)
            .cloned()
            .collect(),
    );
    old.create_schema("knowledge");
    old.dangerous_set_table_name("knowledge._sqlx_migrations");
    old.run(&pool).await.unwrap();
    let store = PostgresKnowledge::new(&Secret::new(db), 4, Duration::from_secs(5)).unwrap();
    let facts = facts();
    let project = facts.rows[0].project_id;
    let k = Knowledge::new(store.clone(), facts);
    let initiator = actor();
    let mut c = k
        .create_candidate_as(project, None, None, initiator.clone())
        .await
        .unwrap();
    c.progress = json!({"run_id":uuid::Uuid::new_v4(),"status":"running"});
    c.updated_at -= chrono::TimeDelta::seconds(121);
    c.execution_input = Some(json!({"version":2,"original":"saved input"}));
    c.batches.push(CandidateBatch {
        id: nexofolio_common::RoundId::new(),
        author: author(),
        state: RoundState::Done,
        commands: vec![Command::PutEndpointNote {
            endpoint: c.base.endpoints[0],
            name: Some("retained".into()),
            purpose: None,
            context: Default::default(),
        }],
    });
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&c).await.unwrap();
    tx.commit().await.unwrap();
    store.migrate().await.unwrap();
    store.migrate().await.unwrap();
    let task = k.task(project, c.id).await.unwrap();
    assert_eq!(task.actor, initiator);
    assert_eq!(task.candidate_id, Some(c.id));
    assert_eq!(task.generation, 1);
    let after = k.candidate(project, c.id).await.unwrap();
    assert_eq!(
        serde_json::to_value(&after).unwrap(),
        serde_json::to_value(&c).unwrap()
    );
    let claimed = k.claim_task(1).await.unwrap().unwrap();
    assert_eq!(claimed.id, c.id);
    assert_eq!(claimed.generation, 2);
    let restored = k.prepare_task(&claimed).await.unwrap();
    assert_eq!(restored.execution_input, c.execution_input);
    assert_eq!(restored.batches.len(), 1);
    assert_eq!(restored.batches[0].commands.len(), 1);
    k.stop_task(project, claimed.id).await.unwrap();
    store.close().await;
    pool.close().await;
}
