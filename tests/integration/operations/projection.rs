use super::*;

#[tokio::test]
#[ignore = "isolated PostgreSQL migration/write-path/read-cost regression"]
async fn projection_upgrade_bounds_and_heartbeat_identity() {
    let h = Harness::start().await;
    let task = h.submit(0, false).await;
    h.wait(task, "done").await;
    let candidate: uuid::Uuid =
        sqlx::query_scalar("SELECT candidate_id FROM knowledge.curation_tasks WHERE id=$1")
            .bind(task)
            .fetch_one(&h.sql)
            .await
            .unwrap();
    let mut tx = h.sql.begin().await.unwrap();
    sqlx::raw_sql("SET LOCAL search_path=knowledge; ALTER TABLE candidates DROP COLUMN operations_checkpoint CASCADE; DROP FUNCTION capture_operations_checkpoint(); DROP FUNCTION operations_checkpoint_summary(jsonb);")
        .execute(&mut *tx).await.unwrap();
    // Existing candidate payload stays byte-for-byte intact and unprojected on upgrade.
    let before: String =
        sqlx::query_scalar("SELECT md5(progress::text) FROM candidates WHERE id=$1")
            .bind(candidate)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../modules/knowledge/adapter/migrations/202610130002_operations_checkpoint.sql"
    ))
    .execute(&mut *tx)
    .await
    .unwrap();
    let absent: Option<Value> =
        sqlx::query_scalar("SELECT operations_checkpoint FROM candidates WHERE id=$1")
            .bind(candidate)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert!(absent.is_none());
    let after: String =
        sqlx::query_scalar("SELECT md5(progress::text) FROM candidates WHERE id=$1")
            .bind(candidate)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(before, after);
    let run = uuid::Uuid::new_v4();
    let progress = json!({"run_id":run,"stage":"fields","updated_at":"2026-10-13T01:00:00Z","completed":3,"total":8,"completed_batches":2,"message":"not exported","activity":vec!["large irrelevant progress";10000]});
    sqlx::query("UPDATE candidates SET progress=$2 WHERE id=$1")
        .bind(candidate)
        .bind(&progress)
        .execute(&mut *tx)
        .await
        .unwrap();
    let summary: Value =
        sqlx::query_scalar("SELECT operations_checkpoint FROM candidates WHERE id=$1")
            .bind(candidate)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(
        summary,
        json!({"run_id":run,"stage":"fields","updated_at":"2026-10-13T01:00:00Z","completed":3,"total":8,"completed_batches":2})
    );
    sqlx::query("UPDATE candidates SET updated_at=now()+interval '10 seconds' WHERE id=$1")
        .bind(candidate)
        .execute(&mut *tx)
        .await
        .unwrap();
    let heartbeat: Value =
        sqlx::query_scalar("SELECT operations_checkpoint FROM candidates WHERE id=$1")
            .bind(candidate)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(summary, heartbeat);
    // Malformed metadata cannot poison a business write or leak free text into the summary.
    for invalid in [
        Value::Null,
        json!([]),
        json!({"run_id":"sk-secret","stage":"secret","completed":{},"total":-1,"updated_at":"password=secret"}),
    ] {
        sqlx::query("UPDATE candidates SET progress=$2 WHERE id=$1")
            .bind(candidate)
            .bind(invalid)
            .execute(&mut *tx)
            .await
            .unwrap();
        let value: Value =
            sqlx::query_scalar("SELECT operations_checkpoint FROM candidates WHERE id=$1")
                .bind(candidate)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert!(value.as_object().unwrap().values().all(Value::is_null));
    }
    sqlx::query("UPDATE candidates SET progress=$2,operations_checkpoint=$3 WHERE id=$1")
        .bind(candidate)
        .bind(&progress)
        .bind(json!({"injected":"x".repeat(10000)}))
        .execute(&mut *tx)
        .await
        .unwrap();
    let size: i32 = sqlx::query_scalar(
        "SELECT pg_column_size(operations_checkpoint) FROM candidates WHERE id=$1",
    )
    .bind(candidate)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert!(size <= 1024);
    h.write("projection-upgrade.json",json!({"old_progress_hash_unchanged":before==after,"old_summary_unknown":absent.is_none(),"summary":summary,"heartbeat_did_not_change_checkpoint":heartbeat==summary,"summary_bytes":size,"invalid_progress_writes_succeeded":3}));
    tx.rollback().await.unwrap();
    h.close().await;
}
