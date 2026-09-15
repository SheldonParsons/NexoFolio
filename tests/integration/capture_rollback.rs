use async_trait::async_trait;
use nexofolio_access::SessionPrincipal;
use nexofolio_contracts::*;
use nexofolio_evidence::{BlobRef, BlobStore};
use nexofolio_infrastructure::{Postgres, PostgresCaptureStore};
use nexofolio_intake::{CaptureAdmissionStore, prepare_capture};
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;
struct FailedVolume;
#[async_trait]
impl BlobStore for FailedVolume {
    async fn put(&self, _: ProjectId, _: Vec<u8>, _: &str) -> Result<BlobRef> {
        Err(Error::Unavailable {
            component: "test_volume_full",
        })
    }
    async fn get(&self, _: ProjectId, _: &str) -> Result<Vec<u8>> {
        Err(Error::Unavailable {
            component: "test_volume_full",
        })
    }
    async fn delete(&self, _: ProjectId, _: &str) -> Result<()> {
        Err(Error::Unavailable {
            component: "test_volume_full",
        })
    }
}
pub async fn verify(
    db: &Postgres,
    sql: &sqlx::PgPool,
    principal: &SessionPrincipal,
    template: &Value,
) {
    const COUNTS: &str = "SELECT (SELECT count(*) FROM ingestion_inbox),(SELECT count(*) FROM capture_events),(SELECT count(*) FROM capture_receipts),(SELECT coalesce(sum(pending),0)::bigint FROM capture_backlog)";
    let before: (i64, i64, i64, i64) = sqlx::query_as(COUNTS).fetch_one(sql).await.unwrap();
    let mut input = template.clone();
    let mut record = input["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "http_exchange")
        .unwrap()
        .clone();
    record["record_id"] = serde_json::json!(Uuid::new_v4());
    record["payload"]["request"]["url"] =
        serde_json::json!("https://api.example.test/rollback-fixture");
    input["records"] = serde_json::json!([record]);
    let batch: CaptureBatch = serde_json::from_value(input).unwrap();
    let prepared = prepare_capture(&batch, 0).unwrap();
    let failed = PostgresCaptureStore::new(db.clone(), Arc::new(FailedVolume));
    assert!(matches!(
        failed
            .admit_captures(
                principal.user_id,
                &principal.instance,
                batch,
                vec![prepared]
            )
            .await,
        Err(Error::Unavailable {
            component: "test_volume_full"
        })
    ));
    let after: (i64, i64, i64, i64) = sqlx::query_as(COUNTS).fetch_one(sql).await.unwrap();
    assert_eq!(
        before, after,
        "volume failure must roll back structure, event, receipt and capacity reservation"
    );
}
