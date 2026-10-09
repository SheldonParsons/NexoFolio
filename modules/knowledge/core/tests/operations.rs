#[path = "operations/support.rs"]
mod support;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
#[tokio::test]
async fn operation_identity_is_transactional_and_idempotent() {
    support::lifecycle(InMemoryKnowledge::new()).await;
}
