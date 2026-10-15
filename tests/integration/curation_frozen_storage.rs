//! Storage invariants exercised through the real knowledge ports and private PostgreSQL.
use super::*;
use std::collections::{BTreeMap, HashMap};

async fn manifest(
    k: &Knowledge<PostgresKnowledge, Facts>,
    c: &Candidate,
    owner: uuid::Uuid,
    rows: &[EndpointSummary],
) -> FrozenManifest {
    let snapshot = uuid::Uuid::new_v4();
    let empty = k
        .put_frozen_chunk(c.project_id, c.id, owner, snapshot, "[]")
        .await
        .unwrap();
    let values = k
        .put_frozen_chunk(
            c.project_id,
            c.id,
            owner,
            snapshot,
            r#"{"version":1,"fields":{},"documents":{}}"#,
        )
        .await
        .unwrap();
    let mut endpoints = HashMap::new();
    for summary in rows.iter().filter(|r| r.project_id == c.project_id) {
        let content=json!({"facts":{"summary":summary,"fields":[],"requests":[],"addresses":[],"aliases":[],"examples":[]},"authentication":[]}).to_string();
        let facts = k
            .put_frozen_chunk(c.project_id, c.id, owner, snapshot, &content)
            .await
            .unwrap();
        endpoints.insert(
            summary.id,
            FrozenEndpoint {
                facts,
                inputs: vec![],
                values: values.clone(),
                documents: BTreeMap::new(),
            },
        );
    }
    FrozenManifest {
        version: 1,
        project: c.project_id,
        candidate: c.id,
        snapshot,
        endpoints,
        links: empty,
    }
}
fn input(m: &FrozenManifest, rows: &[EndpointSummary]) -> Value {
    let mut value = json!({"version":3,"lifecycle_version":0,"manifest":m,"summaries":rows.iter().filter(|r|r.project_id==m.project).collect::<Vec<_>>(),"baseline_batches":[],"legacy":false});
    value["manifest_hash"] = json!(frozen_input_hash(&value));
    value
}
#[tokio::test]
#[ignore = "private PostgreSQL only"]
async fn chunks_fence_owners_publish_atomically_and_reset_only_their_project() {
    let db = fixture::database_url();
    let store =
        PostgresKnowledge::new(&Secret::new(db.clone()), 8, Duration::from_secs(5)).unwrap();
    store.migrate().await.unwrap();
    let facts = facts();
    let p = facts.rows[0].project_id;
    let q = facts.rows[2].project_id;
    let k = Knowledge::new(store.clone(), facts.clone());
    k.submit_task(p, None, None, None, actor()).await.unwrap();
    let old = k.claim_task(1).await.unwrap().unwrap();
    let c = k.prepare_task(&old).await.unwrap();
    let abandoned = manifest(&k, &c, old.run_id.unwrap(), &facts.rows).await;
    assert!(matches!(
        k.frozen_chunk(q, c.id, abandoned.snapshot, &abandoned.links)
            .await,
        Err(KnowledgeError::FrozenInputInvalid)
    ));
    expire(&store, &k.task(p, old.id).await.unwrap()).await;
    let new = k.claim_task(1).await.unwrap().unwrap();
    let resumed = k.prepare_task(&new).await.unwrap();
    assert_eq!(c.id, resumed.id);
    assert!(
        k.put_frozen_chunk(p, c.id, old.run_id.unwrap(), abandoned.snapshot, "late")
            .await
            .is_err()
    );
    assert!(
        k.freeze_candidate_input(p, c.id, old.run_id.unwrap(), input(&abandoned, &facts.rows))
            .await
            .is_err()
    );
    let complete = manifest(&k, &resumed, new.run_id.unwrap(), &facts.rows).await;
    let mut incomplete = complete.clone();
    incomplete.links = FrozenRef::of("missing");
    assert!(matches!(
        k.freeze_candidate_input(
            p,
            c.id,
            new.run_id.unwrap(),
            input(&incomplete, &facts.rows)
        )
        .await,
        Err(KnowledgeError::FrozenInputInvalid)
    ));
    assert!(
        k.candidate(p, c.id)
            .await
            .unwrap()
            .execution_input
            .is_none()
    );
    k.freeze_candidate_input(p, c.id, new.run_id.unwrap(), input(&complete, &facts.rows))
        .await
        .unwrap();
    assert!(
        k.put_frozen_chunk(
            p,
            c.id,
            new.run_id.unwrap(),
            complete.snapshot,
            "after publication"
        )
        .await
        .is_err()
    );
    k.stop_task(p, new.id).await.unwrap();
    assert!(
        k.put_frozen_chunk(
            p,
            c.id,
            new.run_id.unwrap(),
            uuid::Uuid::new_v4(),
            "after stop"
        )
        .await
        .is_err()
    );
    k.submit_task(q, None, None, None, actor()).await.unwrap();
    let other = k.claim_task(1).await.unwrap().unwrap();
    let other_c = k.prepare_task(&other).await.unwrap();
    let other_m = manifest(&k, &other_c, other.run_id.unwrap(), &facts.rows).await;
    k.freeze_candidate_input(
        q,
        other_c.id,
        other.run_id.unwrap(),
        input(&other_m, &facts.rows),
    )
    .await
    .unwrap();
    k.stop_task(q, other.id).await.unwrap();
    let pool = sqlx::PgPool::connect(&db).await.unwrap();
    assert!(sqlx::query("INSERT INTO knowledge.frozen_input_chunks(project_id,candidate_id,snapshot_id,sha256,content) VALUES($1,$2,$3,$4,'[]')").bind(q.to_string().parse::<uuid::Uuid>().unwrap()).bind(c.id).bind(uuid::Uuid::new_v4()).bind(&complete.links.sha256).execute(&pool).await.is_err());
    let mut tx = pool.begin().await.unwrap();
    store.prepare_project_reset(&mut tx, p).await.unwrap();
    store.reset_project_content(&mut tx, p).await.unwrap();
    tx.commit().await.unwrap();
    assert!(
        k.frozen_chunk(p, c.id, complete.snapshot, &complete.links)
            .await
            .is_err()
    );
    assert_eq!(
        k.frozen_chunk(q, other_c.id, other_m.snapshot, &other_m.links)
            .await
            .unwrap(),
        "[]"
    );
    // SQL fault injection, then the normal reader must fail closed on the same reference.
    sqlx::query("UPDATE knowledge.frozen_input_chunks SET content='[1]' WHERE candidate_id=$1 AND sha256=$2").bind(other_c.id).bind(&other_m.links.sha256).execute(&pool).await.unwrap();
    assert!(matches!(
        k.frozen_chunk(q, other_c.id, other_m.snapshot, &other_m.links)
            .await,
        Err(KnowledgeError::FrozenInputInvalid)
    ));
    pool.close().await;
    store.close().await;
}
#[tokio::test]
#[ignore = "fresh private PostgreSQL only"]
async fn additive_migration_preserves_old_input_archive_and_batches() {
    let db = fixture::database_url();
    let options: sqlx::postgres::PgConnectOptions = db.parse().unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_with(options.options([("search_path", "knowledge")]))
        .await
        .unwrap();
    let all = sqlx::migrate!("../../modules/knowledge/adapter/migrations");
    let mut old = sqlx::migrate::Migrator::with_migrations(
        all.iter()
            .filter(|m| m.version < 202610130003)
            .cloned()
            .collect(),
    );
    old.create_schema("knowledge");
    old.dangerous_set_table_name("knowledge._sqlx_migrations");
    old.run(&pool).await.unwrap();
    let store = PostgresKnowledge::new(&Secret::new(db), 4, Duration::from_secs(5)).unwrap();
    let facts = facts();
    let p = facts.rows[0].project_id;
    let k = Knowledge::new(store.clone(), facts);
    let mut c = k.create_candidate(p, None, None).await.unwrap();
    c.execution_input = Some(json!({"version":2,"retained":{"number":123,"text":"original"}}));
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&c).await.unwrap();
    tx.put_value_archive(
        p,
        c.id,
        &nexofolio_contracts::field_values::FieldValueArchive {
            version: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let before: Value = sqlx::query_scalar("SELECT to_jsonb(c) FROM candidates c WHERE id=$1")
        .bind(c.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let archive: String =
        sqlx::query_scalar("SELECT archive FROM field_value_archives WHERE candidate_id=$1")
            .bind(c.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    store.migrate().await.unwrap();
    let mut after: Value = sqlx::query_scalar("SELECT to_jsonb(c) FROM candidates c WHERE id=$1")
        .bind(c.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    for column in [
        "history_pruned_at",
        "execution_released_at",
        "evidence_input",
        "archive_unreferenced_since",
        "archive_reclaimed_at",
    ] {
        assert_eq!(
            after.as_object_mut().unwrap().remove(column),
            Some(Value::Null),
            "additive retention column must not backfill existing data"
        );
    }
    assert_eq!(before, after);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM retention_projects")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0,
        "migration never enables maintenance"
    );
    let after_archive: String =
        sqlx::query_scalar("SELECT archive FROM field_value_archives WHERE candidate_id=$1")
            .bind(c.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(archive, after_archive);
    let chunks: i64 = sqlx::query_scalar("SELECT count(*) FROM frozen_input_chunks")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(chunks, 0);
    pool.close().await;
    store.close().await;
}
