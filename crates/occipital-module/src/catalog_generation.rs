//! When to read the daemon's catalog again.
//!
//! Three readers here keep facts taken from the daemon's `catalog.list`: each
//! runner's build (so its cached `role.describe` is read again after a
//! restart), the modules that declare commands, and the tool-view providers'
//! versions. A module's restart, replacement or new declaration changes the
//! catalog, and nothing else tells this module about it.
//!
//! The daemon's `catalog.watch` pushes the catalog's generation at once and
//! again whenever it changes. [`CatalogGeneration`] follows it on one
//! connection and keeps the latest value, and each reader stamps a [`Checked`]
//! when it reads the catalog. A read is then fresh until the generation moves,
//! with a slow backstop, so the readers list the catalog only when it changed.
//! While the watch is down (no daemon connection, an older daemon, a refused
//! subscription), a read is fresh for [`POLL`] only, as when this module
//! polled.
//!
//! A reconnect starts a new feed whose numbers need not continue the old
//! one's (a restarted daemon counts afresh), so each feed gets its own local
//! number, and a stamp from an earlier feed is never fresh under a later one.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use subc_client_rs::SubcConsumer;
use tokio::sync::watch;
// The runtime's clock, so tests can move it.
use tokio::time::Instant;

/// How long a catalog read is used while the watch is down.
pub const POLL: Duration = Duration::from_secs(10);
/// How long a catalog read is used while the watch is live and the generation
/// hasn't moved: a backstop against a missed push.
pub const BACKSTOP: Duration = Duration::from_secs(300);
/// The longest wait before opening the watch again after it ended or failed.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Where the follower gets its daemon connection.
#[async_trait]
pub trait Connection: Send + Sync + 'static {
    /// The connection, made now if there is none.
    async fn consumer(&self) -> Option<Arc<SubcConsumer>>;
    /// The watch failed on this connection: forget it if it may have closed.
    async fn failed(&self) {}
}

/// A connection that's already open. The client makes it again by itself
/// after it closes.
pub struct Held(pub Arc<SubcConsumer>);

#[async_trait]
impl Connection for Held {
    async fn consumer(&self) -> Option<Arc<SubcConsumer>> {
        Some(self.0.clone())
    }
}

/// One live feed's position: the feed's local number, then the daemon's
/// generation.
type Position = (u64, u64);

/// The daemon catalog's latest generation, as followed by one task.
#[derive(Clone)]
pub struct CatalogGeneration {
    latest: watch::Receiver<Option<Position>>,
}

/// When a reader last read the catalog, and the generation it saw just before.
#[derive(Debug, Clone, Copy)]
pub struct Checked {
    at: Instant,
    position: Option<Position>,
}

impl Default for CatalogGeneration {
    fn default() -> Self {
        Self::unwatched()
    }
}

impl CatalogGeneration {
    /// No watch: every read is fresh for [`POLL`]. For tests, and for a module
    /// with no daemon connection.
    pub fn unwatched() -> Self {
        Self {
            latest: watch::channel(None).1,
        }
    }

    /// Follow `connection`'s catalog generation in a task. The task ends once
    /// every clone of the returned value is dropped.
    pub fn follow(connection: Arc<dyn Connection>) -> Self {
        let (sender, latest) = watch::channel(None);
        tokio::spawn(async move {
            tokio::select! {
                () = run(connection.as_ref(), &sender) => {}
                () = sender.closed() => {}
            }
        });
        Self { latest }
    }

    /// Stamp a catalog read. Take it before reading, so a change during the
    /// read makes the next check read again.
    pub fn check(&self) -> Checked {
        Checked {
            at: Instant::now(),
            position: *self.latest.borrow(),
        }
    }

    /// Whether a read stamped `last` may still be used.
    pub fn fresh(&self, last: Option<&Checked>) -> bool {
        let Some(last) = last else {
            return false;
        };
        match *self.latest.borrow() {
            Some(now) => last.position == Some(now) && last.at.elapsed() < BACKSTOP,
            None => last.at.elapsed() < POLL,
        }
    }

    /// A generation with a hand-set position, for tests.
    #[cfg(test)]
    pub(crate) fn manual() -> (watch::Sender<Option<(u64, u64)>>, Self) {
        let (sender, latest) = watch::channel(None);
        (sender, Self { latest })
    }
}

async fn run(connection: &dyn Connection, latest: &watch::Sender<Option<Position>>) {
    let mut feed = 0u64;
    let mut backoff = Duration::from_secs(1);
    loop {
        if let Some(consumer) = connection.consumer().await {
            match consumer.catalog_watch().await {
                Ok(mut subscription) => {
                    feed += 1;
                    loop {
                        match subscription.next().await {
                            Ok(Some(response)) => {
                                latest.send_replace(Some((feed, response.generation)));
                                backoff = Duration::from_secs(1);
                            }
                            Ok(None) => break,
                            Err(error) => {
                                tracing::debug!(%error, "catalog.watch ended");
                                break;
                            }
                        }
                    }
                }
                Err(error) => {
                    tracing::debug!(%error, "catalog.watch couldn't be opened");
                    connection.failed().await;
                }
            }
        }
        latest.send_replace(None);
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_read_is_fresh_until_the_generation_moves_or_the_backstop_passes() {
        let (set, generation) = CatalogGeneration::manual();
        assert!(!generation.fresh(None), "never read");
        set.send_replace(Some((1, 7)));
        let read = generation.check();
        tokio::time::advance(POLL * 3).await;
        assert!(
            generation.fresh(Some(&read)),
            "unchanged, under the backstop"
        );
        set.send_replace(Some((1, 8)));
        assert!(!generation.fresh(Some(&read)), "the generation moved");
        let read = generation.check();
        tokio::time::advance(BACKSTOP).await;
        assert!(!generation.fresh(Some(&read)), "past the backstop");
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_feed_is_never_taken_for_the_old_one() {
        let (set, generation) = CatalogGeneration::manual();
        set.send_replace(Some((1, 7)));
        let read = generation.check();
        // The daemon restarted and counts from 7 again on a new feed.
        set.send_replace(Some((2, 7)));
        assert!(!generation.fresh(Some(&read)));
    }

    #[tokio::test(start_paused = true)]
    async fn without_the_watch_a_read_is_fresh_for_the_poll_interval() {
        let generation = CatalogGeneration::unwatched();
        let read = generation.check();
        assert!(generation.fresh(Some(&read)));
        tokio::time::advance(POLL).await;
        assert!(!generation.fresh(Some(&read)));
        // A read taken while the watch was live goes stale once it is down, on
        // the poll interval.
        let (set, generation) = CatalogGeneration::manual();
        set.send_replace(Some((1, 3)));
        let read = generation.check();
        set.send_replace(None);
        assert!(generation.fresh(Some(&read)));
        tokio::time::advance(POLL).await;
        assert!(!generation.fresh(Some(&read)));
    }
}
