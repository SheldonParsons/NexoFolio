//! Directory IO; folder design and placement proposals belong to curate.
use super::{Curation, CurationError};
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::RoundState;

impl Curation {
    pub(super) async fn organize_snapshot(
        &self,
        project: ProjectId,
        summaries: &[nexofolio_contracts::endpoint::EndpointSummary],
        job: &super::jobs::Job,
    ) -> Result<(), CurationError> {
        let notes = self.reader.notes(project).await?;
        let catalog = self.reader.catalogue(project).await?;
        let mut placements = Vec::new();
        for endpoint in summaries {
            if let Some(placement) = self.reader.endpoint(endpoint.id).await?.placement {
                placements.push(placement);
            }
        }
        let signature = nexofolio_curate::directory_input_hash(
            summaries,
            &notes,
            &catalog.folders,
            &placements,
        );
        if job.snapshot().directory_input_hash.as_deref() == Some(&signature) {
            job.update(|p| {
                p.directory_reused = true;
                p.folders_saved = Some(catalog.folders.len());
            });
            return Ok(());
        }
        job.update(|p| p.model_calls += 1);
        let started = std::time::Instant::now();
        let result = self
            .organize
            .project_with_snapshot(summaries, &notes, &catalog.folders, &placements)
            .await;
        job.update(|p| p.model_ms += started.elapsed().as_millis() as u64);
        let commands = result?;

        if !commands.is_empty() {
            let round = self
                .knowledge
                .begin(project, self.organize.author())
                .await?;
            match self.knowledge.apply(round, &commands).await {
                Ok(_) => {
                    self.knowledge
                        .finish(
                            round,
                            RoundState::Done,
                            Some(format!("组织 {} 个接口", summaries.len())),
                        )
                        .await?;
                    let updated = self.reader.catalogue(project).await?;
                    job.update(|p| p.folders_saved = Some(updated.folders.len()));
                    let mut seats = Vec::new();
                    for endpoint in summaries {
                        if let Some(placement) = self.reader.endpoint(endpoint.id).await?.placement
                        {
                            seats.push(placement);
                        }
                    }
                    job.update(|p| {
                        p.directory_input_hash = Some(nexofolio_curate::directory_input_hash(
                            summaries,
                            &notes,
                            &updated.folders,
                            &seats,
                        ))
                    });
                    Ok(())
                }
                Err(e) => {
                    let _ = self
                        .knowledge
                        .finish(round, RoundState::Failed, Some("组织失败".into()))
                        .await;
                    Err(e.into())
                }
            }
        } else {
            job.update(|p| {
                p.directory_input_hash = Some(signature);
                p.folders_saved = Some(catalog.folders.len());
            });
            Ok(())
        }
    }
}
