//! Local projection for the running worker; all checkpoints go through knowledge.
use async_trait::async_trait;
use nexofolio_common::{EndpointId, FolderId, ProjectId, RoundId};
use nexofolio_contracts::knowledge::*;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub struct CandidateSession {
    pub id: uuid::Uuid,
    project: ProjectId,
    owner: Option<uuid::Uuid>,
    candidates: Arc<dyn KnowledgeCandidates>,
    view: Mutex<KnowledgeSnapshot>,
}
impl CandidateSession {
    pub fn new(candidate: &Candidate, candidates: Arc<dyn KnowledgeCandidates>) -> Self {
        Self {
            id: candidate.id,
            project: candidate.project_id,
            owner: candidate
                .progress
                .get("run_id")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok()),
            candidates,
            view: Mutex::new(candidate.view.clone()),
        }
    }
    fn check_project(&self, project: ProjectId) -> KnowledgeResult<()> {
        if project == self.project {
            Ok(())
        } else {
            Err(KnowledgeError::WrongProject)
        }
    }
}
fn catalogue(project: ProjectId, snapshot: &KnowledgeSnapshot) -> Catalogue {
    let rows: Vec<_> = snapshot
        .folders
        .iter()
        .map(|f| super::snapshot_tx::row(project, f))
        .collect();
    let mut counts = HashMap::new();
    for placement in &snapshot.placements {
        *counts.entry(placement.folder).or_insert(0) += 1;
    }
    Catalogue {
        project_id: project,
        folders: crate::tree::sorted(&rows, &counts),
        unplaced: snapshot
            .endpoints
            .iter()
            .filter(|id| !snapshot.placements.iter().any(|p| &p.endpoint == *id))
            .count(),
    }
}
#[async_trait]
impl KnowledgeReader for CandidateSession {
    async fn catalogue(&self, project: ProjectId) -> KnowledgeResult<Catalogue> {
        self.check_project(project)?;
        Ok(catalogue(
            project,
            &self.view.lock().expect("candidate view"),
        ))
    }
    async fn folder_endpoints(
        &self,
        project: ProjectId,
        folder: Option<FolderId>,
        page: usize,
        limit: usize,
    ) -> KnowledgeResult<EndpointPage> {
        self.check_project(project)?;
        let view = self.view.lock().expect("candidate view");
        if folder.is_some_and(|id| !view.folders.iter().any(|f| f.id == id)) {
            return Err(KnowledgeError::NoSuchFolder);
        }
        let mut ids: Vec<_> = view
            .endpoints
            .iter()
            .copied()
            .filter(|id| {
                view.placements
                    .iter()
                    .find(|p| p.endpoint == *id)
                    .map(|p| p.folder)
                    == folder
            })
            .collect();
        ids.sort_by_key(|id| id.to_string());
        let total = ids.len();
        let page = page.max(1);
        let limit = limit.max(1);
        Ok(EndpointPage {
            items: ids
                .into_iter()
                .skip(page.saturating_sub(1).saturating_mul(limit))
                .take(limit)
                .collect(),
            total,
            page,
            limit,
        })
    }
    async fn endpoint(&self, endpoint: EndpointId) -> KnowledgeResult<EndpointKnowledge> {
        let view = self.view.lock().expect("candidate view");
        if !view.endpoints.contains(&endpoint) {
            return Err(KnowledgeError::WrongProject);
        }
        let placement = view
            .placements
            .iter()
            .find(|p| p.endpoint == endpoint)
            .cloned();
        let folders = catalogue(self.project, &view).folders;
        let mut path = Vec::new();
        let mut at = placement.as_ref().map(|p| p.folder);
        while let Some(id) = at {
            let Some(folder) = folders.iter().find(|f| f.id == id) else {
                break;
            };
            path.push(folder.clone());
            at = folder.parent;
            if path.len() > MAX_DEPTH {
                return Err(KnowledgeError::InvalidTree("candidate path cycle".into()));
            }
        }
        path.reverse();
        Ok(EndpointKnowledge {
            note: view.notes.iter().find(|n| n.endpoint == endpoint).cloned(),
            placement,
            path,
            fields: view
                .fields
                .iter()
                .filter(|n| n.endpoint == endpoint)
                .cloned()
                .collect(),
            links: view
                .links
                .iter()
                .filter(|l| l.from == endpoint || l.to == endpoint)
                .cloned()
                .collect(),
        })
    }
    async fn notes(&self, project: ProjectId) -> KnowledgeResult<Vec<EndpointNote>> {
        self.check_project(project)?;
        Ok(self.view.lock().expect("candidate view").notes.clone())
    }
    async fn links(&self, project: ProjectId) -> KnowledgeResult<Vec<Link>> {
        self.check_project(project)?;
        Ok(self.view.lock().expect("candidate view").links.clone())
    }
    async fn rounds(&self, project: ProjectId, _: usize) -> KnowledgeResult<Vec<Round>> {
        self.check_project(project)?;
        Ok(vec![])
    }
}
#[async_trait]
impl KnowledgeWriter for CandidateSession {
    async fn begin(&self, project: ProjectId, author: Author) -> KnowledgeResult<RoundId> {
        self.check_project(project)?;
        self.candidates
            .open_candidate_batch(project, self.id, self.owner, author)
            .await
    }
    async fn apply(&self, round: RoundId, commands: &[Command]) -> KnowledgeResult<()> {
        let view = self
            .candidates
            .append_candidate_view(self.project, self.id, self.owner, round, commands)
            .await?;
        *self.view.lock().expect("candidate view") = view;
        Ok(())
    }
    async fn finish(
        &self,
        round: RoundId,
        state: RoundState,
        _: Option<String>,
    ) -> KnowledgeResult<()> {
        self.candidates
            .close_candidate_batch(self.project, self.id, self.owner, round, state)
            .await
    }
    async fn roll_back(&self, _: RoundId) -> KnowledgeResult<()> {
        Err(KnowledgeError::CandidateClosed)
    }
}
