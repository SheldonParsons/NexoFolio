//! Change feeds: how one module tells others what changed, without knowing who listens.
//!
//! The publisher appends events to its own outbox and serves them through
//! [`ChangeFeed`]. Each subscriber stores the last [`Cursor`] it processed and
//! reads on from there, so a failed subscriber simply replays.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Position in a feed. Cursors only grow; [`Cursor::START`] is before the first event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cursor(pub i64);

impl Cursor {
    pub const START: Self = Self(0);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change<E> {
    pub cursor: Cursor,
    pub at: DateTime<Utc>,
    pub event: E,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FeedError {
    /// Retryable: storage is down.
    #[error("change feed unavailable")]
    Unavailable,
}

#[async_trait]
pub trait ChangeFeed<E>: Send + Sync {
    /// Up to `limit` events after `after`, in cursor order. Reading the same
    /// range again returns the same events, and no event may ever appear
    /// behind a cursor a reader has already been given.
    async fn read(&self, after: Cursor, limit: usize) -> Result<Vec<Change<E>>, FeedError>;
}
