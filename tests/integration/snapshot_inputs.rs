//! Storage/input boundary tests; the snapshot is never handed to a live model.
use super::*;
use nexofolio_application::{MaintenanceSources, MaintenanceStore};
use nexofolio_infrastructure::PostgresMaintenance;
pub async fn verify(
    db: &Postgres,
    sql: &sqlx::PgPool,
    capture: &PostgresCaptureStore,
    user: UserId,
    project: ProjectId,
) {
    let store = PostgresMaintenance::new(db.clone());
    let first = store
        .start(
            user,
            project,
            &StartMaintenance {
                request_id: Uuid::new_v4(),
            },
        )
        .await
        .unwrap();
    let frozen = store.snapshot(user, project, first.id).await.unwrap();
    let inputs = frozen.inputs.as_ref().unwrap();
    assert!(!inputs.observations.is_empty());
    assert!(!inputs.sources.is_empty());
    let source = inputs
        .sources
        .iter()
        .find(|s| s.raw_hash.is_some())
        .unwrap();
    let original = MaintenanceSources::observation(capture, source)
        .await
        .unwrap();
    let saved: (Option<Value>, String) =
        sqlx::query_as("SELECT context,captured_at::text FROM capture_events WHERE id=$1")
            .bind(source.event_id)
            .fetch_one(sql)
            .await
            .unwrap();
    // Derived mutation proves the reader uses frozen metadata, not whatever a live lookup returns later.
    sqlx::query("UPDATE capture_events SET context=NULL,captured_at=clock_timestamp() WHERE id=$1")
        .bind(source.event_id)
        .execute(sql)
        .await
        .unwrap();
    assert_eq!(
        original,
        MaintenanceSources::observation(capture, source)
            .await
            .unwrap()
    );
    sqlx::query(
        "UPDATE capture_events SET context=$2,captured_at=$3::text::timestamptz WHERE id=$1",
    )
    .bind(source.event_id)
    .bind(saved.0)
    .bind(saved.1)
    .execute(sql)
    .await
    .unwrap();
    let mut missing = source.clone();
    missing.raw_hash = None;
    assert!(
        MaintenanceSources::observation(capture, &missing)
            .await
            .is_err()
    );
    let before = serde_json::to_value(&frozen).unwrap();
    let first_revision: Uuid = frozen.interfaces[0].environments[0]
        .revision_id
        .to_string()
        .parse()
        .unwrap();
    let fact = frozen.facts[0].id;
    let old_definition: Value =
        sqlx::query_scalar("SELECT definition FROM interface_observed_revisions WHERE id=$1")
            .bind(first_revision)
            .fetch_one(sql)
            .await
            .unwrap();
    let old_fact: Value = sqlx::query_scalar("SELECT data FROM evidence_facts WHERE id=$1")
        .bind(fact)
        .fetch_one(sql)
        .await
        .unwrap();
    // An atomic writer deliberately does not take the evidence lock: a single MVCC cut
    // must still keep interface definitions and facts from different commits from mixing.
    let pool = sql.clone();
    let writer = tokio::spawn(async move {
        for n in 1..=24_i64 {
            let mut tx = pool.begin().await.unwrap();
            sqlx::query("UPDATE interface_observed_revisions SET definition=definition||jsonb_build_object('_snapshot_probe',$2::bigint) WHERE id=$1").bind(first_revision).bind(n).execute(&mut *tx).await.unwrap();
            tokio::task::yield_now().await;
            sqlx::query("UPDATE evidence_facts SET data=data||jsonb_build_object('_snapshot_probe',$2::bigint) WHERE id=$1").bind(fact).bind(n).execute(&mut *tx).await.unwrap();
            tx.commit().await.unwrap();
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });
    for _ in 0..6 {
        let run = store
            .start(
                user,
                project,
                &StartMaintenance {
                    request_id: Uuid::new_v4(),
                },
            )
            .await
            .unwrap();
        let snapshot = store.snapshot(user, project, run.id).await.unwrap();
        let definition = &snapshot
            .interfaces
            .iter()
            .flat_map(|i| &i.environments)
            .find(|e| e.revision_id.to_string() == first_revision.to_string())
            .unwrap()
            .definition;
        let data = &snapshot.facts.iter().find(|f| f.id == fact).unwrap().data;
        assert_eq!(
            definition["_snapshot_probe"], data["_snapshot_probe"],
            "mixed database cuts"
        );
        sqlx::query(
            "UPDATE maintenance_runs SET status='failed',error_code='TEST_ONLY' WHERE id=$1",
        )
        .bind(run.id)
        .execute(sql)
        .await
        .unwrap();
    }
    writer.await.unwrap();
    sqlx::query("UPDATE interface_observed_revisions SET definition=$2 WHERE id=$1")
        .bind(first_revision)
        .bind(old_definition)
        .execute(sql)
        .await
        .unwrap();
    sqlx::query("UPDATE evidence_facts SET data=$2 WHERE id=$1")
        .bind(fact)
        .bind(old_fact)
        .execute(sql)
        .await
        .unwrap();
    assert_eq!(
        before,
        serde_json::to_value(store.snapshot(user, project, first.id).await.unwrap()).unwrap()
    );
    let mut legacy = before.clone();
    legacy.as_object_mut().unwrap().remove("inputs");
    let legacy: KnowledgeSnapshot = serde_json::from_value(legacy).unwrap();
    assert!(legacy.inputs.is_none());
    sqlx::query("UPDATE maintenance_runs SET status='failed',error_code='TEST_ONLY' WHERE id=$1")
        .bind(first.id)
        .execute(sql)
        .await
        .unwrap();
}
