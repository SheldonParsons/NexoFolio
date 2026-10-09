#[path = "../../modules/knowledge/core/tests/operations/support.rs"]
mod support;
use nexofolio_access_contracts::ActorNames;
use nexofolio_common::Secret;
use nexofolio_knowledge_adapter::PostgresKnowledge;
use std::time::Duration;
#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn stored_operation_identity_is_atomic_and_survives_reader_reload() {
    let url = Secret::new(std::env::var("TEST_DATABASE_URL").expect("isolated database required"));
    let store = PostgresKnowledge::new(&url, 4, Duration::from_secs(5)).unwrap();
    store.migrate().await.unwrap();
    support::lifecycle(store).await;
}
#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn historical_names_resolve_known_disabled_users_without_returning_private_profile_data() {
    let url = Secret::new(std::env::var("TEST_DATABASE_URL").expect("isolated database required"));
    let database =
        nexofolio_access_adapter::Postgres::new(&url, 4, Duration::from_secs(5)).unwrap();
    database.migrate().await.unwrap();
    let access =
        nexofolio_access_adapter::PostgresAccess::new(database, &Secret::new("ab".repeat(32)))
            .unwrap();
    let pool = sqlx::PgPool::connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO access.users(id,instance,external_id,account,display_name,enabled) VALUES ($1,'fixture',$2,$2,'原名称',false)").bind(id).bind(id.to_string()).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO access.profile_preferences(user_id,display_name) VALUES ($1,'现有资料名称')",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    let typed = id.to_string().parse().unwrap();
    let names = access.actor_names(&[typed]).await.unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].id, typed);
    assert_eq!(names[0].display_name, "现有资料名称");
    assert!(access.actor_names(&[]).await.unwrap().is_empty());
}
