#[path = "candidates/support.rs"]
#[allow(dead_code)]
mod support;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;

#[tokio::test]
async fn pending_entry_search_is_not_limited_by_history_pagination() {
    support::review_entry_selection(InMemoryKnowledge::new()).await;
}
