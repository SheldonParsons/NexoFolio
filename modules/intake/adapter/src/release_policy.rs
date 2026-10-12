use super::PostgresLedger;
use nexofolio_intake_contracts::LedgerError;

impl PostgresLedger {
    /// Atomic, durable high-water mark shared by API instances. No capture records are changed.
    pub async fn remember_fetcher_policy(
        &self,
        revision: i64,
        minimum: &[i32; 4],
        latest: &[i32; 4],
        fingerprint: &str,
    ) -> Result<bool, LedgerError> {
        let result = sqlx::query(
            "INSERT INTO fetcher_release_policy(id,revision,minimum,latest,fingerprint) VALUES(1,$1,$2,$3,$4) \
             ON CONFLICT(id) DO UPDATE SET revision=EXCLUDED.revision,minimum=EXCLUDED.minimum,latest=EXCLUDED.latest,fingerprint=EXCLUDED.fingerprint \
             WHERE EXCLUDED.revision >= fetcher_release_policy.revision \
               AND EXCLUDED.minimum >= fetcher_release_policy.minimum \
               AND EXCLUDED.latest >= fetcher_release_policy.latest \
               AND (EXCLUDED.revision > fetcher_release_policy.revision OR EXCLUDED.fingerprint = fetcher_release_policy.fingerprint \
                    OR (EXCLUDED.revision=0 AND fetcher_release_policy.revision=0))",
        )
        .bind(revision)
        .bind(minimum.as_slice())
        .bind(latest.as_slice())
        .bind(fingerprint)
        .execute(&self.pool)
        .await
        .map_err(|_| LedgerError::Unavailable)?;
        Ok(result.rows_affected() == 1)
    }
}
