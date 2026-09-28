//! `PostgresLedger` against the shared batch ledger conformance suite.
use std::time::Duration;

use nexofolio_common::Secret;
use nexofolio_intake_adapter::PostgresLedger;
use nexofolio_intake_contracts::testing::batch_ledger_conformance;

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL; run with --ignored explicitly"]
async fn postgres_ledger_passes_batch_ledger_conformance() {
    let url = std::env::var("TEST_DATABASE_URL").expect("set an isolated TEST_DATABASE_URL");
    let ledger = PostgresLedger::new(&Secret::new(&url), 2, Duration::from_secs(2)).unwrap();
    ledger.migrate().await.unwrap();
    ledger.migrate().await.expect("migrating twice is harmless");
    batch_ledger_conformance(&ledger).await;
    ledger.close().await;
}
