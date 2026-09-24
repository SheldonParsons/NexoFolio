use std::fmt::Debug;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::DateTime;

use crate::feed::{Change, ChangeFeed, Cursor, FeedError};

/// Fake publisher: an in-memory outbox.
pub struct InMemoryFeed<E> {
    changes: Mutex<Vec<Change<E>>>,
}

impl<E> Default for InMemoryFeed<E> {
    fn default() -> Self {
        Self {
            changes: Mutex::new(Vec::new()),
        }
    }
}

impl<E> InMemoryFeed<E> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish(&self, event: E) -> Cursor {
        let mut changes = self.changes.lock().expect("fake lock");
        let cursor = Cursor(changes.len() as i64 + 1);
        changes.push(Change {
            cursor,
            // One second per event keeps the fake deterministic.
            at: DateTime::from_timestamp(1_790_000_000 + cursor.0, 0).expect("valid timestamp"),
            event,
        });
        cursor
    }
}

#[async_trait]
impl<E: Clone + Send + Sync> ChangeFeed<E> for InMemoryFeed<E> {
    async fn read(&self, after: Cursor, limit: usize) -> Result<Vec<Change<E>>, FeedError> {
        let changes = self.changes.lock().expect("fake lock");
        Ok(changes
            .iter()
            .filter(|change| change.cursor > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

/// Behaviour every [`ChangeFeed`] must show. The feed must already hold at
/// least three events and receive no new ones meanwhile. Panics on the first
/// violation.
pub async fn change_feed_conformance<E, F>(feed: &F)
where
    E: Clone + PartialEq + Debug + Send + Sync,
    F: ChangeFeed<E>,
{
    let all = feed
        .read(Cursor::START, usize::MAX)
        .await
        .expect("read all");
    assert!(all.len() >= 3, "seed at least three events");
    assert!(
        all.windows(2).all(|pair| pair[0].cursor < pair[1].cursor),
        "cursor order"
    );
    assert!(all[0].cursor > Cursor::START);

    assert_eq!(
        feed.read(Cursor::START, usize::MAX).await.expect("reread"),
        all,
        "reading again is stable"
    );
    assert_eq!(
        feed.read(Cursor::START, 2).await.expect("limited"),
        all[..2],
        "limit"
    );
    assert_eq!(
        feed.read(all[0].cursor, usize::MAX).await.expect("resume"),
        all[1..],
        "resume after a cursor"
    );
    let last = all.last().expect("non-empty").cursor;
    assert!(
        feed.read(last, usize::MAX).await.expect("tail").is_empty(),
        "nothing after the last cursor"
    );
    assert!(
        feed.read(Cursor::START, 0).await.expect("zero").is_empty(),
        "zero limit"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_passes_change_feed_conformance() {
        let feed = InMemoryFeed::new();
        for event in ["a", "b", "c"] {
            feed.publish(event.to_owned());
        }
        change_feed_conformance::<String, _>(&feed).await;
    }
}
