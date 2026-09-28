//! `PostgresObserve` against the shared observe store conformance suite.
use std::time::Duration;

use nexofolio_common::Secret;
use nexofolio_observe_adapter::PostgresObserve;
use nexofolio_observe_contracts::testing::observe_store_conformance;

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL; run with --ignored explicitly"]
async fn postgres_observe_passes_observe_store_conformance() {
    let url = std::env::var("TEST_DATABASE_URL").expect("set an isolated TEST_DATABASE_URL");
    let store = PostgresObserve::new(&Secret::new(&url), 2, Duration::from_secs(2)).unwrap();
    store.migrate().await.unwrap();
    store.migrate().await.expect("migrating twice is harmless");
    observe_store_conformance(&store).await;
    store.close().await;
}
