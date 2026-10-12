//! Read-time status overlays never rewrite historical executor checkpoints.
use nexofolio_contracts::knowledge::CandidateSummary;

pub(super) fn lifecycle_status(row: &mut CandidateSummary, current_version: u64) {
    if !row.progress.is_object() {
        row.progress = serde_json::json!({});
    }
    let version = row.progress["lifecycle_version"].as_u64().unwrap_or(0);
    let stale = version != current_version;
    row.progress["lifecycle_stale"] = serde_json::json!(stale);
    if stale {
        row.progress["resume_available"] = serde_json::json!(false);
    }
}
