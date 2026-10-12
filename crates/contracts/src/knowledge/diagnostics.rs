//! Diagnostic read projection. It never carries frozen observations, commands, or knowledge text.
use super::*;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticBatch {
    pub id: RoundId,
    pub author: Author,
    pub state: RoundState,
    pub commands: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateDiagnostics {
    pub summary: CandidateSummary,
    pub batches: Vec<DiagnosticBatch>,
    pub operations: Vec<CandidateOperation>,
}
impl From<Candidate> for CandidateDiagnostics {
    fn from(c: Candidate) -> Self {
        Self {
            summary: c.summary(),
            batches: c
                .batches
                .into_iter()
                .map(|b| DiagnosticBatch {
                    id: b.id,
                    author: b.author,
                    state: b.state,
                    commands: b.commands.len(),
                })
                .collect(),
            operations: c.operations,
        }
    }
}
