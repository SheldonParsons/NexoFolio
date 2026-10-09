#[path = "../../modules/knowledge/core/tests/values/support.rs"]
mod support;
#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn postgres_value_archive_is_immutable_and_preserves_large_numbers() {
    let url = nexofolio_common::Secret::new(std::env::var("TEST_DATABASE_URL").unwrap());
    let store = nexofolio_knowledge_adapter::PostgresKnowledge::new(
        &url,
        4,
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    store.migrate().await.unwrap();
    support::archive_read_write(store).await;
}
