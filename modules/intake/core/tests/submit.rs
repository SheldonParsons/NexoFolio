//! `Intake::submit` against the in-memory access, observe and ledger fakes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{EnvironmentId, ProjectId};
use nexofolio_contracts::observation::{Body, Content, Fact};
use nexofolio_contracts::testing::{InMemoryScope, RecordingSink, ScopeFixture};
use nexofolio_intake::{Clock, CollectError, Intake, Limits, purge_expired};
use nexofolio_intake_contracts::testing::InMemoryLedger;
use nexofolio_intake_contracts::{BatchLedger, LedgerEntry, LedgerError, RejectReason};
use serde_json::{Value, json};
use uuid::Uuid;

/// Seconds since a fixed start; tests move it by hand.
#[derive(Default)]
struct ManualClock(AtomicI64);

impl ManualClock {
    fn advance(&self, seconds: i64) {
        self.0.fetch_add(seconds, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000 + self.0.load(Ordering::SeqCst), 0).unwrap()
    }
}

/// A ledger whose writes can fail while reads keep working: the crash window
/// between delivering to observe and recording the batch.
#[derive(Default)]
struct FlakyLedger {
    inner: InMemoryLedger,
    fail_writes: AtomicBool,
}

#[async_trait]
impl BatchLedger for FlakyLedger {
    async fn find(&self, batch_id: Uuid) -> Result<Option<LedgerEntry>, LedgerError> {
        self.inner.find(batch_id).await
    }
    async fn record(&self, entry: &LedgerEntry) -> Result<(), LedgerError> {
        match self.fail_writes.load(Ordering::SeqCst) {
            true => Err(LedgerError::Unavailable),
            false => self.inner.record(entry).await,
        }
    }
    async fn purge(&self, before: DateTime<Utc>) -> Result<u64, LedgerError> {
        self.inner.purge(before).await
    }
}

struct World {
    scope: Arc<InMemoryScope>,
    sink: Arc<RecordingSink>,
    ledger: Arc<FlakyLedger>,
    clock: Arc<ManualClock>,
    intake: Intake,
    project: ProjectId,
    environment: EnvironmentId,
}

impl World {
    async fn new(limits: Limits) -> Self {
        let scope = Arc::new(InMemoryScope::new());
        let project = scope.create_project().await;
        let environment = scope.create_environment(project, "test").await;
        let sink = Arc::new(RecordingSink::new());
        let ledger = Arc::new(FlakyLedger::default());
        let clock = Arc::new(ManualClock::default());
        let intake = Intake::new(
            scope.clone(),
            sink.clone(),
            ledger.clone(),
            clock.clone(),
            limits,
        );
        Self {
            scope,
            sink,
            ledger,
            clock,
            intake,
            project,
            environment,
        }
    }

    async fn default() -> Self {
        Self::new(Limits::default()).await
    }

    fn batch(&self, records: Vec<Value>) -> Value {
        json!({
            "batch_id": Uuid::new_v4(),
            "platform": "nexofolio-fetcher",
            "target": {
                "project_id": self.project,
                "environment": {"id": self.environment},
                "site": {"origin": "https://shop.example.com", "prefix": "/"}
            },
            "records": records
        })
    }

    async fn submit(
        &self,
        batch: &Value,
    ) -> Result<nexofolio_intake_contracts::Receipt, CollectError> {
        self.intake
            .submit(&serde_json::to_vec(batch).unwrap())
            .await
    }
}

fn exchange() -> Value {
    json!({
        "id": Uuid::new_v4(),
        "kind": "http_exchange",
        "version": 1,
        "observed_at": "2026-09-24T08:00:00Z",
        "context": {"page_url": "https://shop.example.com/#/order", "seq": 1},
        "payload": {
            "request": {
                "method": "GET",
                "url": "https://shop.example.com/api/order/1001",
                "body": {"state": "none"}
            },
            "response": {
                "status": 200,
                "body": {"state": "full", "media_type": "application/json", "encoding": "utf8", "content": "{\"id\":1001}"}
            }
        }
    })
}

fn declaration() -> Value {
    json!({
        "id": Uuid::new_v4(),
        "kind": "http_declaration",
        "version": 1,
        "observed_at": "2026-09-24T08:00:00+08:00",
        "payload": {
            "method": "GET",
            "path": "/order/{id}",
            "request": {"path_params": {"id": {"required": true, "schema": {"type": "string"}}}},
            "responses": {"200": {"media_type": "application/json", "schema": {"type": "object"}}}
        }
    })
}

#[tokio::test]
async fn accepted_records_become_canonical_observations() {
    let world = World::default().await;
    let batch = world.batch(vec![exchange(), declaration()]);
    let receipt = world.submit(&batch).await.unwrap();
    assert_eq!((receipt.accepted, receipt.rejected.len()), (2, 0));

    let observations = world.sink.accepted();
    assert_eq!(observations.len(), 2);
    for observation in &observations {
        assert_eq!(observation.key.batch_id, receipt.batch_id);
        assert_eq!(observation.project_id, world.project);
        assert_eq!(observation.environment_id, Some(world.environment));
        assert_eq!(observation.source.platform, "nexofolio-fetcher");
        assert_eq!(
            observation
                .site
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("https://shop.example.com/")
        );
    }
    let exchange = observations
        .iter()
        .find(|o| matches!(o.fact, Fact::Exchange(_)))
        .unwrap();
    assert_eq!(
        exchange.context.as_ref().unwrap()["seq"],
        1,
        "context kept as sent"
    );
    assert_eq!(
        world.scope.site_annotations(world.environment).len(),
        1,
        "site recorded on the environment"
    );
}

#[tokio::test]
async fn a_retry_gets_the_first_receipt_and_counts_once() {
    let world = World::default().await;
    let mut bad = exchange();
    bad["kind"] = json!("page_context");
    let batch = world.batch(vec![exchange(), bad]);
    let first = world.submit(&batch).await.unwrap();
    assert_eq!(first.rejected[0].reason, RejectReason::UnsupportedKind);

    // Same content, different key order and formatting.
    let reordered: Value =
        serde_json::from_str(&serde_json::to_string_pretty(&batch).unwrap()).unwrap();
    let mut text = serde_json::to_string_pretty(&reordered).unwrap();
    text.insert_str(1, "\n\n");
    assert_eq!(
        world.intake.submit(text.as_bytes()).await,
        Ok(first.clone())
    );
    assert_eq!(world.sink.accepted().len(), 1);

    let mut changed = batch.clone();
    changed["records"][0]["observed_at"] = json!("2026-09-24T09:00:00Z");
    assert_eq!(
        world.submit(&changed).await,
        Err(CollectError::BatchIdReused)
    );
}

#[tokio::test]
async fn delivery_failures_are_retryable_and_count_once() {
    let world = World::default().await;
    let batch = world.batch(vec![exchange()]);

    world.sink.fail_next(1);
    let error = world.submit(&batch).await.unwrap_err();
    assert_eq!(error.code(), "UNAVAILABLE");
    assert!(world.ledger.inner.is_empty(), "nothing recorded");

    // Delivered, then the ledger write fails: observe already has it.
    world.ledger.fail_writes.store(true, Ordering::SeqCst);
    assert_eq!(
        world.submit(&batch).await.unwrap_err().code(),
        "UNAVAILABLE"
    );
    assert_eq!(world.sink.accepted().len(), 1);

    world.ledger.fail_writes.store(false, Ordering::SeqCst);
    let receipt = world.submit(&batch).await.unwrap();
    assert_eq!(receipt.accepted, 1);
    assert_eq!(
        world.sink.accepted().len(),
        1,
        "the sink ignores repeated keys"
    );
    assert_eq!(world.ledger.inner.len(), 1);
}

#[tokio::test]
async fn platforms_are_rate_limited_and_retries_count() {
    let world = World::new(Limits {
        rate_per_minute: 60,
        burst: 2,
    })
    .await;
    let batch = world.batch(vec![exchange()]);
    world.submit(&batch).await.unwrap();
    world.submit(&batch).await.unwrap();
    assert_eq!(
        world.submit(&batch).await,
        Err(CollectError::RateLimited {
            retry_after_secs: 1
        })
    );
    world.clock.advance(1);
    assert!(world.submit(&batch).await.is_ok());
}

#[tokio::test]
async fn targets_must_exist() {
    let world = World::default().await;
    let mut batch = world.batch(vec![exchange()]);
    batch["target"]["environment"] = json!({"id": EnvironmentId::new()});
    assert_eq!(
        world.submit(&batch).await,
        Err(CollectError::UnknownEnvironment)
    );

    batch["batch_id"] = json!(Uuid::new_v4());
    batch["target"]["project_id"] = json!(ProjectId::new());
    assert_eq!(
        world.submit(&batch).await,
        Err(CollectError::UnknownProject)
    );
    assert!(world.sink.accepted().is_empty());
}

#[tokio::test]
async fn environments_named_by_the_client_are_created() {
    let world = World::default().await;
    let mut batch = world.batch(vec![exchange()]);
    batch["target"]["environment"] = json!({"name": "预发布"});
    world.submit(&batch).await.unwrap();
    assert_eq!(
        world.scope.environment_names(world.project).await,
        ["test", "预发布"]
    );
    let observation = &world.sink.accepted()[0];
    assert_ne!(observation.environment_id, Some(world.environment));
    assert!(observation.environment_id.is_some());
}

#[tokio::test]
async fn declarations_alone_may_omit_the_environment() {
    let world = World::default().await;
    let mut batch = world.batch(vec![declaration()]);
    batch["target"] = json!({
        "project_id": world.project,
        "source_url": "https://shop.example.com/v3/api-docs"
    });
    world.submit(&batch).await.unwrap();
    let observation = &world.sink.accepted()[0];
    assert_eq!(observation.environment_id, None);
    assert_eq!(
        observation.source.source_url.as_deref(),
        Some("https://shop.example.com/v3/api-docs")
    );

    batch["batch_id"] = json!(Uuid::new_v4());
    batch["records"] = json!([declaration(), exchange()]);
    assert_eq!(
        world.submit(&batch).await.unwrap_err().code(),
        "INVALID_BATCH"
    );
}

#[tokio::test]
async fn bad_records_reject_only_themselves() {
    let world = World::default().await;
    let good = exchange();
    let mut repeated = exchange();
    repeated["id"] = good["id"].clone();
    let mut new_version = exchange();
    new_version["version"] = json!(2);
    let mut no_id = exchange();
    no_id.as_object_mut().unwrap().remove("id");
    let mut huge = exchange();
    huge["payload"]["response"]["body"]["content"] =
        json!("x".repeat(nexofolio_intake::MAX_RECORD_BYTES));
    let mut binary = exchange();
    binary["payload"]["response"]["body"] =
        json!({"state": "full", "encoding": "base64", "content": "AAEC/w=="});

    let batch = world.batch(vec![good, repeated, new_version, no_id, huge, binary]);
    let receipt = world.submit(&batch).await.unwrap();
    let rejected: Vec<_> = receipt
        .rejected
        .iter()
        .map(|r| (r.index, r.reason, r.id.is_empty()))
        .collect();
    assert_eq!(
        rejected,
        [
            (1, RejectReason::InvalidRecord, false),
            (2, RejectReason::UnsupportedVersion, false),
            (3, RejectReason::InvalidRecord, true),
            (4, RejectReason::RecordTooLarge, false),
        ]
    );
    assert_eq!(receipt.accepted, 2);
    let decoded = world
        .sink
        .accepted()
        .into_iter()
        .find_map(|o| match o.fact {
            Fact::Exchange(exchange) => match exchange.response?.body {
                Body::Full {
                    content: Content::Binary(bytes),
                    ..
                } => Some(bytes),
                _ => None,
            },
            Fact::Declaration(_) => None,
        });
    assert_eq!(decoded, Some(vec![0, 1, 2, 255]), "base64 is decoded");
}

#[tokio::test]
async fn whole_batch_errors() {
    let world = World::default().await;
    assert_eq!(
        world
            .intake
            .submit(&vec![b' '; nexofolio_intake::MAX_BATCH_BYTES + 1])
            .await,
        Err(CollectError::BatchTooLarge)
    );
    assert_eq!(
        world.intake.submit(b"{").await.unwrap_err().code(),
        "INVALID_BATCH"
    );
    let mut batch = world.batch(vec![exchange()]);
    batch["batch_id"] = json!("not-a-uuid");
    assert_eq!(
        world.submit(&batch).await.unwrap_err().code(),
        "INVALID_BATCH"
    );
    batch["batch_id"] = json!(Uuid::new_v4());
    batch["platform_version"] = Value::Null;
    assert_eq!(
        world.submit(&batch).await.unwrap_err().code(),
        "INVALID_BATCH",
        "null is not an absent optional field"
    );
}

#[tokio::test]
async fn processed_batches_expire_after_seven_days() {
    let world = World::default().await;
    world.submit(&world.batch(vec![exchange()])).await.unwrap();
    let now = world.clock.now();
    assert_eq!(purge_expired(world.ledger.as_ref(), now).await, Ok(0));
    let later = now + nexofolio_intake::RETENTION + chrono::TimeDelta::seconds(1);
    assert_eq!(purge_expired(world.ledger.as_ref(), later).await, Ok(1));
}
