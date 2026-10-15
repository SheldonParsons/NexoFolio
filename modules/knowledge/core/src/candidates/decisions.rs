use super::{snapshot, validation::load};
use crate::{Clock, Knowledge, store_error};
use nexofolio_common::{ProjectId, RoundId};
use nexofolio_contracts::{endpoint::EndpointReader, knowledge::*};
use nexofolio_knowledge_contracts::KnowledgeStore;

impl<S: KnowledgeStore, R: EndpointReader, C: Clock> Knowledge<S, R, C> {
    pub(super) async fn apply_proposal_as(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        actor: CandidateActor,
        observer: Option<&dyn ApplyObserver>,
    ) -> KnowledgeResult<Vec<RoundId>> {
        let started = std::time::Instant::now();
        let notify = |stage, processed, total| {
            if let Some(observer) = observer {
                observer.update(ApplyProgress {
                    stage,
                    processed,
                    total,
                });
            }
        };
        notify(ApplyStage::Waiting, 0, 0);
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let locked = started.elapsed();
        notify(ApplyStage::Validating, 0, 0);
        let mut candidate = tx
            .candidate_application(project, id)
            .await
            .map_err(store_error)?
            .ok_or(KnowledgeError::NoSuchCandidate)?;
        if candidate.state == CandidateState::Applied {
            return Ok(candidate.applied_rounds);
        }
        if !matches!(
            candidate.state,
            CandidateState::Ready | CandidateState::Partial
        ) {
            return Err(KnowledgeError::CandidateClosed);
        }
        if let Some(input) = &candidate.execution_input {
            super::frozen::validate(tx.as_mut(), &candidate, input).await?;
        }
        let owned: Vec<_> = self
            .endpoints
            .list(project)
            .await
            .map_err(|_| KnowledgeError::Unavailable)?
            .into_iter()
            .map(|e| e.id)
            .collect();
        let current = snapshot::capture(tx.as_mut(), project, owned.clone()).await?;
        if current.lifecycle.version != candidate.base.lifecycle.version {
            return Err(KnowledgeError::LifecycleStale);
        }
        if snapshot::hash(&current) != candidate.base_hash {
            return Err(KnowledgeError::CandidateStale);
        }
        let lifecycle =
            crate::lifecycle::resolve(&self.endpoints, current.lifecycle.clone()).await?;
        super::validation::lifecycle_view(&candidate.view, &lifecycle)?;
        tx.prepare_application(project, &current)
            .await
            .map_err(store_error)?;
        let validated = started.elapsed();
        let total = candidate.batches.iter().map(|b| b.commands.len()).sum();
        let mut processed = 0;
        notify(ApplyStage::Writing, 0, total);
        let mut applied = Vec::new();
        if candidate.target.is_none() {
            for p in &current.placements {
                if let Some(decision) = lifecycle
                    .endpoint(p.endpoint)
                    .or_else(|| lifecycle.folder(p.folder))
                {
                    let round = Round {
                        id: RoundId::new(),
                        project_id: project,
                        author: Author::Person {
                            user: decision
                                .actor
                                .user_id()
                                .ok_or(KnowledgeError::WrongProject)?,
                        },
                        started_at: self.clock.now(),
                        finished_at: None,
                        state: RoundState::Running,
                        summary: None,
                    };
                    tx.create_round(&round).await.map_err(store_error)?;
                    crate::apply::one(
                        tx.as_mut(),
                        project,
                        round.id,
                        0,
                        &round.author,
                        self.clock.now(),
                        &Command::Unplace {
                            endpoint: p.endpoint,
                        },
                    )
                    .await?;
                    tx.finish_round(
                        round.id,
                        RoundState::Done,
                        Some("解除废弃内容的旧归属"),
                        self.clock.now(),
                    )
                    .await
                    .map_err(store_error)?;
                    applied.push(round.id);
                }
            }
        }

        for group in candidate.batches.iter().filter(|b| !b.commands.is_empty()) {
            let round = Round {
                id: RoundId::new(),
                project_id: project,
                author: group.author.clone(),
                started_at: self.clock.now(),
                finished_at: None,
                state: RoundState::Running,
                summary: None,
            };
            tx.create_round(&round).await.map_err(store_error)?;
            let mut writes = group.clone();
            writes.id = round.id;
            super::validation::commands_observed(
                tx.as_mut(),
                project,
                &owned,
                &writes,
                self.clock.now(),
                0,
                &mut |count| notify(ApplyStage::Writing, processed + count, total),
            )
            .await?;
            processed += writes.commands.len();
            tx.flush_application().await.map_err(store_error)?;
            tx.finish_round(
                round.id,
                RoundState::Done,
                Some(&format!("应用候选整理 {}", id)),
                self.clock.now(),
            )
            .await
            .map_err(store_error)?;
            applied.push(round.id);
        }
        let mut effective = snapshot::capture(tx.as_mut(), project, owned).await?;
        lifecycle.project_snapshot(&mut effective);
        super::validation::valid_names(&effective)?;
        if candidate.target.is_none() && candidate.state == CandidateState::Ready {
            let mut reconciled = current.lifecycle.clone();
            reconciled.reconciled_version = reconciled.version;
            tx.put_lifecycle(project, &reconciled)
                .await
                .map_err(store_error)?;
        }
        candidate.state = CandidateState::Applied;
        candidate.reviewed_by = actor.user_id();
        candidate.applied_rounds = applied.clone();
        candidate.updated_at = self.clock.now();
        candidate.operations.push(CandidateOperation::new(
            CandidateAction::Apply,
            actor,
            candidate.updated_at,
        ));
        tx.close_candidate_tasks(project, id, self.clock.now())
            .await
            .map_err(store_error)?;
        tx.put_candidate_decision(&candidate)
            .await
            .map_err(store_error)?;
        let written = started.elapsed();
        notify(ApplyStage::Committing, processed, total);
        tx.commit().await.map_err(store_error)?;
        notify(ApplyStage::Committed, processed, total);
        tracing::info!(project=%project, candidate=%id, lock_ms=locked.as_millis() as u64,
            validate_ms=(validated-locked).as_millis() as u64, write_ms=(written-validated).as_millis() as u64,
            commit_ms=(started.elapsed()-written).as_millis() as u64, total_ms=started.elapsed().as_millis() as u64,
            commands=total, "candidate apply timing");
        Ok(applied)
    }
    pub(super) async fn discard_proposal_as(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        actor: CandidateActor,
    ) -> KnowledgeResult<()> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let mut candidate = load(tx.as_mut(), project, id).await?;
        if candidate.state == CandidateState::Discarded {
            return Ok(());
        }
        if !matches!(
            candidate.state,
            CandidateState::Ready | CandidateState::Partial | CandidateState::Interrupted
        ) {
            return Err(KnowledgeError::CandidateClosed);
        }
        candidate.state = CandidateState::Discarded;
        candidate.reviewed_by = actor.user_id();
        candidate.updated_at = self.clock.now();
        candidate.operations.push(CandidateOperation::new(
            CandidateAction::Discard,
            actor,
            candidate.updated_at,
        ));
        tx.close_candidate_tasks(project, id, self.clock.now())
            .await
            .map_err(store_error)?;
        tx.put_candidate(&candidate).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }
}
