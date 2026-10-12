//! Isolated PostgreSQL + real API processes; the model is a local fixture.
mod curation_restart {
    pub mod fixture;
    pub mod process;
}
#[path = "../../modules/knowledge/core/tests/recovery/support.rs"]
mod recovery_contract;
use nexofolio_common::Secret;
use nexofolio_knowledge_adapter::PostgresKnowledge;
use std::time::Duration;

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL; never use a live service database"]
async fn postgres_rechecks_stop_heartbeat_and_exclusive_ownership() {
    let url = Secret::new(curation_restart::fixture::database_url());
    let store = PostgresKnowledge::new(&url, 8, Duration::from_secs(5)).unwrap();
    store.migrate().await.unwrap();
    recovery_contract::retained_work_and_single_owner(store.clone()).await;
    recovery_contract::manual_stop_wins_after_scan(store.clone()).await;
    recovery_contract::renewed_heartbeat_wins_after_scan(store.clone()).await;
    recovery_contract::missing_input_and_changed_baseline_are_retained_and_blocked(store.clone())
        .await;
    recovery_contract::scan_pages_without_loading_or_resuming_closed_tasks(store.clone()).await;
    store.close().await;
}

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL; starts and kills only fixture API processes"]
async fn killed_api_is_resumed_once_by_two_new_api_processes() {
    curation_restart::process::run().await;
}
