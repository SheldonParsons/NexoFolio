//! Every collect v1 batch fixture goes through `Intake::submit`, so the serde
//! types and the JSON Schema cannot drift apart: valid fixtures are accepted
//! in full, invalid ones fail with the outcome their name describes.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use nexofolio_common::EnvironmentId;
use nexofolio_contracts::scope::{
    CollectTarget, EnvironmentSelector, ResolvedTarget, ScopeError, TargetResolver,
};
use nexofolio_contracts::testing::RecordingSink;
use nexofolio_intake::{Intake, Limits, SystemClock};
use nexofolio_intake_contracts::RejectReason;
use nexofolio_intake_contracts::testing::InMemoryLedger;

/// Fixtures use made-up project and environment IDs; attribution is not
/// what this test is about.
struct AcceptAnyTarget;

#[async_trait]
impl TargetResolver for AcceptAnyTarget {
    async fn resolve(&self, target: &CollectTarget) -> Result<ResolvedTarget, ScopeError> {
        Ok(ResolvedTarget {
            project_id: target.project_id,
            environment_id: target.environment.as_ref().map(|selector| match selector {
                EnvironmentSelector::Id(id) => *id,
                EnvironmentSelector::Name(_) => EnvironmentId::new(),
            }),
            site: target.site.clone(),
            source_url: target.source_url.clone(),
        })
    }
}

enum Expected {
    Batch(&'static str),
    /// Exactly one record is rejected, for this reason; the rest are accepted.
    Record(RejectReason),
}

fn expected(name: &str) -> Expected {
    use Expected::{Batch, Record};
    match name {
        "empty-records"
        | "too-many-records"
        | "missing-platform"
        | "missing-target"
        | "platform-with-spaces"
        | "environment-with-id-and-name"
        | "environment-name-with-surrounding-space"
        | "site-origin-with-path"
        | "exchange-without-environment" => Batch("INVALID_BATCH"),
        "unknown-kind" => Record(RejectReason::UnsupportedKind),
        "unsupported-version" => Record(RejectReason::UnsupportedVersion),
        "declaration-bad-response-code"
        | "declaration-path-with-query"
        | "declaration-with-context"
        | "declaration-with-exchange-payload"
        | "full-body-without-content"
        | "header-entry-not-a-pair"
        | "lowercase-method"
        | "none-body-with-content"
        | "relative-request-url"
        | "response-without-status"
        | "timestamp-without-timezone"
        | "unknown-body-state"
        | "unknown-context-field"
        | "unknown-record-field" => Record(RejectReason::InvalidRecord),
        other => panic!("add an expected outcome for fixture {other}"),
    }
}

fn fixtures(outcome: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../contracts/collect/v1/fixtures/batch")
        .join(outcome);
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no {outcome} fixtures");
    paths
}

fn intake(sink: Arc<RecordingSink>) -> Intake {
    Intake::new(
        Arc::new(AcceptAnyTarget),
        sink,
        Arc::new(InMemoryLedger::new()),
        Arc::new(SystemClock),
        Limits {
            rate_per_minute: 10_000,
            burst: 10_000,
        },
    )
}

#[tokio::test]
async fn valid_fixtures_are_accepted_in_full() {
    for path in fixtures("valid") {
        let raw = fs::read(&path).unwrap();
        let sink = Arc::new(RecordingSink::new());
        let receipt = intake(sink.clone())
            .submit(&raw)
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
        let records = serde_json::from_slice::<serde_json::Value>(&raw).unwrap()["records"]
            .as_array()
            .unwrap()
            .len();
        assert!(
            receipt.rejected.is_empty(),
            "{}: {:?}",
            path.display(),
            receipt.rejected
        );
        assert_eq!(receipt.accepted, records, "{}", path.display());
        assert_eq!(sink.accepted().len(), records, "{}", path.display());
    }
}

#[tokio::test]
async fn invalid_fixtures_fail_as_named() {
    for path in fixtures("invalid") {
        let name = path.file_stem().unwrap().to_str().unwrap();
        let raw = fs::read(&path).unwrap();
        let result = intake(Arc::new(RecordingSink::new())).submit(&raw).await;
        match expected(name) {
            Expected::Batch(code) => match result {
                Err(error) => assert_eq!(error.code(), code, "{name}: {error}"),
                Ok(receipt) => panic!("{name}: accepted as {receipt:?}"),
            },
            Expected::Record(reason) => {
                let receipt = result.unwrap_or_else(|e| panic!("{name}: {e:?}"));
                let reasons: Vec<_> = receipt.rejected.iter().map(|r| r.reason).collect();
                assert_eq!(reasons, [reason], "{name}: {:?}", receipt.rejected);
            }
        }
    }
}
