use nexofolio_access_adapter::Postgres;
use nexofolio_common::DatabaseProbe;
use nexofolio_common::{Error, Secret};
use std::time::Duration;

#[tokio::test]
async fn malformed_database_url_is_redacted_and_unavailable_database_fails() {
    let error = Postgres::new(
        &Secret::new("invalid://private-password"),
        1,
        Duration::from_millis(50),
    )
    .err()
    .unwrap();
    assert!(!error.to_string().contains("private-password"));
    let unavailable = Postgres::new(
        &Secret::new("postgres://unused@127.0.0.1:1/unavailable"),
        1,
        Duration::from_millis(50),
    )
    .unwrap();
    assert_eq!(
        unavailable.check().await,
        Err(Error::Unavailable {
            component: "database"
        })
    );
    unavailable.close().await;
}

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL; run with --ignored explicitly"]
async fn real_postgres_connects_and_migrations_are_repeatable() {
    let url = std::env::var("TEST_DATABASE_URL").expect("set an isolated TEST_DATABASE_URL");
    let database = Postgres::new(&Secret::new(url), 2, Duration::from_secs(2)).unwrap();
    database.check().await.unwrap();
    database.migrate().await.unwrap();
    database.migrate().await.unwrap();
    let pool = sqlx::PgPool::connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT tablename FROM pg_tables WHERE schemaname='access' ORDER BY tablename",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        tables,
        vec![
            "_sqlx_migrations",
            "environment_names",
            "environment_sites",
            "environments",
            "internal_sessions",
            "login_audit",
            "projects",
            "sites",
            "user_project_access",
            "users"
        ]
    );
    let public: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE schemaname='public'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(public, 0, "access keeps nothing in public");
    pool.close().await;
    database.close().await;
}
