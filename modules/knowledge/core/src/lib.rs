//! Knowledge: the catalogue tree, where endpoints sit, and the words written
//! about them.
//!
//! Observe owns what traffic proves. This owns what people and curation say.
//! Nothing here is inferred from a single call: every value arrives as a
//! [`Command`], carries an [`Author`], and belongs to a [`Round`] so it can be
//! undone.
//!
//! Two rules are enforced here rather than in SQL, because both need the whole
//! tree to check:
//!
//! - depth never exceeds [`MAX_DEPTH`]
//! - a folder is never its own ancestor
//!
//! 待分类 is not stored. [`Knowledge::catalogue`] asks observe for the project's
//! endpoints and subtracts the placed ones, so an endpoint is browsable the
//! moment observe accepts it, with no write here and nothing to go wrong.

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{EndpointId, ProjectId, RoundId};
use nexofolio_contracts::{
    endpoint::EndpointReader,
    knowledge::{
        Author, Command, KnowledgeError, KnowledgeResult, KnowledgeWriter, Round, RoundState,
    },
};
use nexofolio_knowledge_contracts::{KnowledgeStore, StoreError};

mod apply;
mod candidates;
mod lifecycle;
mod protection;
mod read;
mod tree;
mod value_reads;
pub use candidates::CandidateSession;

/// Reads the wall clock. Mirrors intake's: `chrono` runs without its `clock`
/// feature here, and tests need to decide what "now" is.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from(std::time::SystemTime::now())
    }
}

/// Knowledge over a store and observe's endpoint reader.
///
/// The reader is needed for one thing only: knowing which endpoints exist, so
/// the unplaced ones can be counted. Knowledge never reads facts or samples.
pub struct Knowledge<S, R, C = SystemClock> {
    store: S,
    endpoints: R,
    clock: C,
}

impl<S, R> Knowledge<S, R, SystemClock> {
    pub fn new(store: S, endpoints: R) -> Self {
        Self {
            store,
            endpoints,
            clock: SystemClock,
        }
    }
}

impl<S, R, C> Knowledge<S, R, C> {
    pub fn with_clock(store: S, endpoints: R, clock: C) -> Self {
        Self {
            store,
            endpoints,
            clock,
        }
    }
}

fn store_error(error: StoreError) -> KnowledgeError {
    match error {
        StoreError::Unavailable => KnowledgeError::Unavailable,
        StoreError::HistoryExpired => KnowledgeError::HistoryExpired,
    }
}

#[async_trait]
impl<S, R, C> KnowledgeWriter for Knowledge<S, R, C>
where
    S: KnowledgeStore,
    R: EndpointReader,
    C: Clock,
{
    async fn begin(&self, project: ProjectId, author: Author) -> KnowledgeResult<RoundId> {
        let round = Round {
            id: RoundId::new(),
            project_id: project,
            author,
            started_at: self.clock.now(),
            finished_at: None,
            state: RoundState::Running,
            summary: None,
        };
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.create_round(&round).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(round.id)
    }

    async fn apply(&self, round: RoundId, commands: &[Command]) -> KnowledgeResult<()> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        let Some(open) = tx.round(round).await.map_err(store_error)? else {
            return Err(KnowledgeError::NoSuchRound);
        };
        // A finished round is what rollback replays; appending to it would
        // leave writes that no undo covers.
        if open.state != RoundState::Running {
            return Err(KnowledgeError::RoundClosed);
        }
        // Tree edits of one project must not interleave with another round's.
        tx.lock_project(open.project_id)
            .await
            .map_err(store_error)?;

        // Commands may only touch endpoints of the round's own project.
        let owned: HashSet<EndpointId> = self
            .endpoints
            .list(open.project_id)
            .await
            .map_err(|_| KnowledgeError::Unavailable)?
            .into_iter()
            .map(|e| e.id)
            .collect();

        let existing = tx.revisions(round).await.map_err(store_error)?;
        let mut seq = existing.iter().map(|r| r.seq).max().unwrap_or(-1) + 1;
        let at = self.clock.now();

        for command in commands {
            for endpoint in touched(command) {
                if !owned.contains(&endpoint) {
                    return Err(KnowledgeError::WrongProject);
                }
            }
            apply::one(
                tx.as_mut(),
                open.project_id,
                round,
                seq,
                &open.author,
                at,
                command,
            )
            .await?;
            seq += 1;
        }

        if commands
            .iter()
            .any(|c| matches!(c, Command::PutFolder { .. }))
        {
            let lifecycle = tx.lifecycle(open.project_id).await.map_err(store_error)?;
            lifecycle::validate_folder_names(tx.as_mut(), open.project_id, &lifecycle).await?;
        }

        tx.commit().await.map_err(store_error)
    }

    async fn finish(
        &self,
        round: RoundId,
        state: RoundState,
        summary: Option<String>,
    ) -> KnowledgeResult<()> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        let Some(open) = tx.round(round).await.map_err(store_error)? else {
            return Err(KnowledgeError::NoSuchRound);
        };
        // Finishing twice would overwrite how the round ended, including a
        // rollback.
        if open.state != RoundState::Running {
            return Err(KnowledgeError::RoundClosed);
        }
        tx.finish_round(round, state, summary.as_deref(), self.clock.now())
            .await
            .map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }

    async fn roll_back(&self, round: RoundId) -> KnowledgeResult<()> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        let Some(open) = tx.round(round).await.map_err(store_error)? else {
            return Err(KnowledgeError::NoSuchRound);
        };
        // Undoing a rollback is not an undo: the revisions have already been
        // replayed, and replaying them again would restore values that later
        // rounds have since changed.
        if open.state == RoundState::RolledBack {
            return Err(KnowledgeError::RoundClosed);
        }
        tx.lock_project(open.project_id)
            .await
            .map_err(store_error)?;

        // Newest write first, so two commands on the same row undo in order.
        for revision in tx.revisions(round).await.map_err(store_error)? {
            tx.restore(open.project_id, &revision.target, revision.before.as_ref())
                .await
                .map_err(store_error)?;
        }
        let lifecycle = tx.lifecycle(open.project_id).await.map_err(store_error)?;
        lifecycle::validate_folder_names(tx.as_mut(), open.project_id, &lifecycle).await?;
        tx.finish_round(round, RoundState::RolledBack, None, self.clock.now())
            .await
            .map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }
}

/// Endpoints a command writes to, so they can be checked against the project.
fn touched(command: &Command) -> Vec<EndpointId> {
    match command {
        Command::Place { endpoint, .. }
        | Command::Unplace { endpoint }
        | Command::PutEndpointNote { endpoint, .. }
        | Command::PutFieldNote { endpoint, .. }
        | Command::RemoveFieldNote { endpoint, .. } => vec![*endpoint],
        Command::PutLink { from, to, .. } => vec![*from, *to],
        Command::PutFolder { .. } | Command::RemoveFolder { .. } | Command::RemoveLink { .. } => {
            Vec::new()
        }
    }
}
