use chrono::{DateTime, Utc};
use serde::Serialize;

/// Provenance applies to every field in value. Failed samples retain their original timestamp.
#[derive(Clone, Serialize)]
pub(crate) struct Snapshot<T> {
    pub value: Option<T>,
    pub sampled_at: Option<DateTime<Utc>>,
    pub attempted_at: Option<DateTime<Utc>>,
    pub source: &'static str,
    pub scope: &'static str,
    pub stale_after_seconds: i64,
    pub error: Option<&'static str>,
}
impl<T> Snapshot<T> {
    pub fn new(source: &'static str, scope: &'static str, stale_after_seconds: i64) -> Self {
        Self {
            value: None,
            sampled_at: None,
            attempted_at: None,
            source,
            scope,
            stale_after_seconds,
            error: None,
        }
    }
    pub fn record(&mut self, value: Result<T, &'static str>, now: DateTime<Utc>) {
        self.attempted_at = Some(now);
        match value {
            Ok(value) => {
                self.value = Some(value);
                self.sampled_at = Some(now);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}
