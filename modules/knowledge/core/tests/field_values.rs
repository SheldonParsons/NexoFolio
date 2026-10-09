#[path = "values/support.rs"]
mod support;
#[tokio::test]
async fn immutable_values_survive_apply_with_exact_types_paging_and_project_scope() {
    support::archive_read_write(nexofolio_knowledge_contracts::testing::InMemoryKnowledge::new())
        .await;
}
