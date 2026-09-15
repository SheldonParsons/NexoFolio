use nexofolio_contracts::Result;
use nexofolio_knowledge::{ObservationProcessor, ProcessingResult, extract_observed};
use std::sync::Arc;

pub struct ProcessingService {
    processor: Arc<dyn ObservationProcessor>,
}
impl ProcessingService {
    pub fn new(processor: Arc<dyn ObservationProcessor>) -> Self {
        Self { processor }
    }
    pub async fn process_one(&self) -> Result<Option<ProcessingResult>> {
        let Some(claim) = self.processor.claim().await? else {
            return Ok(None);
        };
        let definition = match extract_observed(&claim.raw) {
            Ok(d) => d,
            Err(e) => {
                self.processor.fail(&claim, "INVALID_OBSERVATION").await?;
                return Err(e);
            }
        };
        match self.processor.finish(&claim, definition).await {
            Ok(result) => Ok(Some(result)),
            Err(e) => {
                let _ = self.processor.fail(&claim, "DOCUMENT_COMMIT_FAILED").await;
                Err(e)
            }
        }
    }
}
