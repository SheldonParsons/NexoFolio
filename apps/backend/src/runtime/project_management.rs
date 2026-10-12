//! Composition of module-owned maintenance operations in one transaction.
use super::curation::Curation;
use nexofolio_access_adapter::PostgresAccess;
use nexofolio_access_contracts::SessionPrincipal;
use nexofolio_common::{Error, ProjectId, Result};
use nexofolio_contracts::project_lifecycle::ProjectContentCounts;
use nexofolio_knowledge_adapter::PostgresKnowledge;
use nexofolio_observe_adapter::PostgresObserve;
use std::sync::Arc;

pub(crate) struct ProjectAdministration {
    pub access: Arc<PostgresAccess>,
    pub knowledge: PostgresKnowledge,
    pub observe: PostgresObserve,
    pub curation: Curation,
}

impl ProjectAdministration {
    pub async fn contents(
        &self,
        principal: &SessionPrincipal,
        project: ProjectId,
        confirmation: Option<&str>,
    ) -> Result<ProjectContentCounts> {
        let mut tx = self
            .access
            .begin_project_reset(principal, project, confirmation)
            .await?;
        let mut counts = self
            .knowledge
            .prepare_project_reset(tx.connection(), project)
            .await?;
        let observed = self
            .observe
            .prepare_project_reset(tx.connection(), project)
            .await?;
        counts.interfaces = observed.interfaces;
        counts.structures = observed.structures;
        if self.curation.has_running_project(project) {
            counts.running_curations = counts.running_curations.max(1);
        }
        if confirmation.is_some() {
            if counts.running_curations > 0 {
                return Err(Error::Conflict);
            }
            self.knowledge
                .reset_project_content(tx.connection(), project)
                .await?;
            self.observe
                .reset_project_content(tx.connection(), project)
                .await?;
            let previous_run = self.curation.progress(project).run_id;
            tx.commit().await?;
            self.curation.forget_project(project, previous_run);
        }
        Ok(counts)
    }
}
