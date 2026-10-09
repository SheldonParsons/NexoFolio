//! Migration conservatively reconstructs ownership, never inventing manual authority.
use nexofolio_common::Secret;
use serde_json::json;
use sqlx::Connection;

#[tokio::test]
#[ignore = "needs isolated TEST_DATABASE_URL"]
async fn legacy_folder_author_backfill_protects_manual_and_unknown_histories() {
    let url = Secret::new(std::env::var("TEST_DATABASE_URL").expect("isolated database required"));
    let mut db = sqlx::PgConnection::connect(url.expose()).await.unwrap();
    sqlx::raw_sql("CREATE TEMP TABLE folders (id uuid PRIMARY KEY);
        CREATE TEMP TABLE rounds (id integer PRIMARY KEY,author jsonb,started_at timestamptz,state text);
        CREATE TEMP TABLE revisions (round_id integer,seq integer,target jsonb);
        INSERT INTO folders VALUES ('00000000-0000-0000-0000-000000000001'),('00000000-0000-0000-0000-000000000002'),('00000000-0000-0000-0000-000000000003');
        INSERT INTO rounds VALUES
        (1,'{\"by\":\"curate\",\"pass\":\"organize\",\"model\":null}','2026-09-29','done'),
        (2,'{\"by\":\"person\",\"user\":\"00000000-0000-0000-0000-000000000009\"}','2026-09-30','done'),
        (3,'{\"by\":\"curate\",\"pass\":\"organize\",\"model\":null}','2026-10-01','done');")
        .execute(&mut db).await.unwrap();
    for (round, folder) in [(1, 1), (1, 2), (2, 2), (3, 2)] {
        let target = json!({"of":"folder","id":format!("00000000-0000-0000-0000-{folder:012}")});
        sqlx::query("INSERT INTO revisions VALUES ($1,$2,$3)")
            .bind(round)
            .bind(folder)
            .bind(target)
            .execute(&mut db)
            .await
            .unwrap();
    }
    sqlx::raw_sql(include_str!(
        "../../migrations/202610070003_folder_author.sql"
    ))
    .execute(&mut db)
    .await
    .unwrap();
    let values: Vec<Option<serde_json::Value>> =
        sqlx::query_scalar("SELECT author FROM folders ORDER BY id")
            .fetch_all(&mut db)
            .await
            .unwrap();
    assert_eq!(values[0].as_ref().unwrap()["by"], "curate");
    assert!(
        values[1].is_none(),
        "a later automatic round must not erase evidence of human history"
    );
    assert!(
        values[2].is_none(),
        "no history means unknown, not generated"
    );
}
