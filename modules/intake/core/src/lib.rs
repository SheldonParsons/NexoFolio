//! Intake: accepts collect v1 batches (`contracts/collect/v1`), checks them,
//! turns every good record into a [`CanonicalObservation`] and hands the batch
//! to observe.
//!
//! Everything it touches is a port injected by `apps/backend`: the target
//! resolver (access), the observation sink (observe), the batch ledger (intake
//! adapter) and a clock. [`Intake::submit`] is the whole public surface.
//!
//! Delivery is at least once: a crash between handing the batch to the sink
//! and recording it in the ledger makes the client retry, and the sink ignores
//! observation keys it has already seen. No locks, no "in progress" state.

mod hash;
mod limiter;
mod wire;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use chrono::{DateTime, TimeDelta, Utc};
use nexofolio_contracts::observation::{
    CanonicalObservation, ObservationKey, ObservationSink, Source,
};
use nexofolio_contracts::scope::{ScopeError, TargetResolver};
use nexofolio_intake_contracts::{
    BatchLedger, LedgerEntry, LedgerError, Receipt, RejectReason, Rejection,
};
use serde_json::Value;
use uuid::Uuid;

pub use limiter::Limits;
pub use wire::{MAX_BATCH_BYTES, MAX_RECORD_BYTES, MAX_RECORDS};

/// How long a processed batch is remembered for idempotent retries.
pub const RETENTION: TimeDelta = TimeDelta::days(7);

/// Longest `message` in a receipt rejection.
const MAX_MESSAGE_CHARS: usize = 1024;

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from(SystemTime::now())
    }
}

/// Why a whole batch was refused. Each maps to one error code of the contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CollectError {
    #[error("{0}")]
    InvalidBatch(String),
    #[error("batch exceeds {MAX_BATCH_BYTES} bytes")]
    BatchTooLarge,
    #[error("unknown project")]
    UnknownProject,
    #[error("environment does not belong to the project")]
    UnknownEnvironment,
    #[error("batch_id was already used for different content")]
    BatchIdReused,
    #[error("too many batches from this platform")]
    RateLimited { retry_after_secs: u64 },
    /// Retryable. Names the dependency that failed, for logs only.
    #[error("temporarily unavailable")]
    Unavailable(&'static str),
}

impl CollectError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidBatch(_) => "INVALID_BATCH",
            Self::BatchTooLarge => "BATCH_TOO_LARGE",
            Self::UnknownProject => "UNKNOWN_PROJECT",
            Self::UnknownEnvironment => "UNKNOWN_ENVIRONMENT",
            Self::BatchIdReused => "BATCH_ID_REUSED",
            Self::RateLimited { .. } => "RATE_LIMITED",
            Self::Unavailable(_) => "UNAVAILABLE",
        }
    }
}

pub struct Intake {
    resolver: Arc<dyn TargetResolver>,
    sink: Arc<dyn ObservationSink>,
    ledger: Arc<dyn BatchLedger>,
    clock: Arc<dyn Clock>,
    limiter: limiter::RateLimiter,
}

/// What the log line of one batch may contain: identifiers and counts only.
#[derive(Default)]
struct Trace {
    batch_id: Option<Uuid>,
    platform: Option<String>,
    records: usize,
    replayed: bool,
}

impl Intake {
    pub fn new(
        resolver: Arc<dyn TargetResolver>,
        sink: Arc<dyn ObservationSink>,
        ledger: Arc<dyn BatchLedger>,
        clock: Arc<dyn Clock>,
        limits: Limits,
    ) -> Self {
        Self {
            resolver,
            sink,
            ledger,
            clock,
            limiter: limiter::RateLimiter::new(limits),
        }
    }

    /// Processes one request body. Retrying the same batch is always safe.
    pub async fn submit(&self, raw: &[u8]) -> Result<Receipt, CollectError> {
        let started = Instant::now();
        let mut trace = Trace::default();
        let result = self.process(raw, &mut trace).await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let batch_id = trace.batch_id.map(|id| id.to_string());
        let platform = trace.platform.as_deref();
        match &result {
            Ok(receipt) => {
                let reasons: Vec<&str> = receipt.rejected.iter().map(|r| r.reason.code()).collect();
                tracing::info!(
                    batch_id,
                    platform,
                    records = trace.records,
                    accepted = receipt.accepted,
                    rejected = receipt.rejected.len(),
                    reasons = ?reasons,
                    replayed = trace.replayed,
                    elapsed_ms,
                    "collect_batch_processed"
                );
            }
            Err(CollectError::Unavailable(component)) => tracing::warn!(
                batch_id,
                platform,
                component,
                elapsed_ms,
                "collect_batch_unavailable"
            ),
            Err(error) => tracing::info!(
                batch_id,
                platform,
                code = error.code(),
                elapsed_ms,
                "collect_batch_refused"
            ),
        }
        result
    }

    async fn process(&self, raw: &[u8], trace: &mut Trace) -> Result<Receipt, CollectError> {
        if raw.len() > MAX_BATCH_BYTES {
            return Err(CollectError::BatchTooLarge);
        }
        let value: Value = serde_json::from_slice(raw)
            .map_err(|e| CollectError::InvalidBatch(format!("body is not JSON: {e}")))?;
        let platform = wire::platform_of(&value).map_err(CollectError::InvalidBatch)?;
        trace.platform = Some(platform.to_owned());

        // Before the ledger, so retries count too; clients wait and resend.
        let now = self.clock.now();
        self.limiter
            .take(platform, now)
            .map_err(|retry_after_secs| CollectError::RateLimited { retry_after_secs })?;

        let content_hash = hash::content_hash(&value);
        let batch = wire::parse_batch(value).map_err(CollectError::InvalidBatch)?;
        trace.batch_id = Some(batch.batch_id);
        trace.records = batch.records.len();

        match self.ledger.find(batch.batch_id).await {
            Ok(Some(entry)) if entry.content_hash == content_hash => {
                trace.replayed = true;
                return Ok(entry.receipt);
            }
            Ok(Some(_)) => return Err(CollectError::BatchIdReused),
            Ok(None) => {}
            Err(LedgerError::Unavailable) => return Err(CollectError::Unavailable("batch_ledger")),
        }

        let target = self
            .resolver
            .resolve(&batch.target)
            .await
            .map_err(|error| match error {
                ScopeError::UnknownProject => CollectError::UnknownProject,
                ScopeError::UnknownEnvironment => CollectError::UnknownEnvironment,
                ScopeError::InvalidEnvironmentName(_) | ScopeError::InvalidSite(_) => {
                    CollectError::InvalidBatch(error.to_string())
                }
                ScopeError::Unavailable => CollectError::Unavailable("target_resolver"),
            })?;

        let source = Source {
            platform: batch.platform.clone(),
            platform_version: batch.platform_version,
            source_url: target.source_url.clone(),
        };
        let mut observations = Vec::with_capacity(batch.records.len());
        let mut rejected = Vec::new();
        let mut seen = HashSet::new();
        for (index, raw_record) in batch.records.iter().enumerate() {
            let parsed =
                wire::parse_record(raw_record).and_then(|record| match seen.insert(record.id) {
                    true => Ok(record),
                    false => Err((
                        RejectReason::InvalidRecord,
                        "id repeats an earlier record of this batch".into(),
                    )),
                });
            match parsed {
                Ok(record) => observations.push(CanonicalObservation {
                    key: ObservationKey {
                        batch_id: batch.batch_id,
                        record_id: record.id,
                    },
                    project_id: target.project_id,
                    environment_id: target.environment_id,
                    site: target.site.clone(),
                    source: source.clone(),
                    observed_at: record.observed_at,
                    fact: record.fact,
                    context: record.context,
                }),
                Err((reason, message)) => rejected.push(Rejection {
                    index,
                    id: wire::raw_id(raw_record),
                    reason,
                    message: Some(message.chars().take(MAX_MESSAGE_CHARS).collect()),
                }),
            }
        }

        let receipt = Receipt {
            batch_id: batch.batch_id,
            accepted: observations.len(),
            rejected,
        };
        if !observations.is_empty() {
            self.sink
                .accept(observations)
                .await
                .map_err(|_| CollectError::Unavailable("observation_sink"))?;
        }
        self.ledger
            .record(&LedgerEntry {
                batch_id: batch.batch_id,
                content_hash,
                platform: batch.platform,
                receipt: receipt.clone(),
                recorded_at: now,
            })
            .await
            .map_err(|_| CollectError::Unavailable("batch_ledger"))?;
        Ok(receipt)
    }
}

/// Forgets batches older than [`RETENTION`]; run periodically by the worker.
pub async fn purge_expired(
    ledger: &dyn BatchLedger,
    now: DateTime<Utc>,
) -> Result<u64, LedgerError> {
    ledger.purge(now - RETENTION).await
}
