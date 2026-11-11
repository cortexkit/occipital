//! Keeping each call's pin as its runner stored it, while pages are built.
//!
//! An app asks for a call's records by its call key, and the join needs the
//! call's pin from the runner (tool-display §6), never from the app or the
//! write. The app holds a call key only because a page carried it, and every
//! page is built here from the runner's read, so building the page is where
//! the pin is learned. It's kept in the display store, since the app may ask
//! about a chat it cached long before this process started.
//!
//! A call is filed under the runner that minted its key: the page's runner,
//! or for a call moved here from another runner with its history, the one
//! its `call_carrier` names. Providers wrote its records under that runner,
//! so the join finds them there. A call seen in another session follows it,
//! as when another runner imports the chat and gives it a new session id.
//!
//! Page building never waits on this: sightings go to a task through a
//! bounded queue, and one that doesn't fit is dropped. A dropped call is kept
//! the next time a page carrying it is built.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};

use occipital_app_reader::tool_times::CallIdentity;
use tokio::sync::mpsc;

use crate::key_digest::{key_digest, KeyDigest};

use super::store::{now_ms, DisplayStore};
use super::views::Fetcher;

/// How many pages' sightings wait to be kept before more are dropped.
const QUEUE: usize = 256;

/// How many calls are remembered as already kept, so a page read again and
/// again costs no store write. Past this the memory starts over.
const REMEMBERED: usize = 100_000;

/// The calls one built page carries: its session, and each call's minting
/// runner, key and pin.
struct Sighting {
    session: String,
    calls: Vec<(String, String, String)>,
}

/// Where page building reports calls, once the display store is open.
pub type CallPinSlot = Arc<OnceLock<CallPins>>;

pub struct CallPins {
    queue: mpsc::Sender<Sighting>,
    /// The digest of `(minting runner, call key, session)` of each call already kept. The
    /// minting runner is the one that gave the call its key: the call's carrier when it
    /// moved from another runner, else the runner whose page carries it.
    kept: Mutex<HashSet<KeyDigest>>,
}

impl CallPins {
    /// Report the calls a page of `runner`'s `session` carries. Calls already
    /// kept in that session are skipped; never waits.
    pub fn seen(&self, runner: &str, session: &str, calls: Vec<CallIdentity>) {
        let Ok(mut kept) = self.kept.lock() else {
            return;
        };
        let fresh: Vec<(String, String, String)> = calls
            .into_iter()
            .map(|call| {
                let minted_by = call.carrier.unwrap_or_else(|| runner.to_owned());
                (minted_by, call.call_key, call.schema_pin)
            })
            .filter(|(minted_by, call_key, _)| {
                !kept.contains(&key_digest(&[minted_by, call_key, session]))
            })
            .collect();
        if fresh.is_empty() {
            return;
        }
        if kept.len() + fresh.len() > REMEMBERED {
            kept.clear();
        }
        // Remembered only once queued, so a dropped sighting is reported again.
        let remembered: Vec<KeyDigest> = fresh
            .iter()
            .map(|(minted_by, call_key, _)| key_digest(&[minted_by, call_key, session]))
            .collect();
        let sighting = Sighting {
            session: session.to_owned(),
            calls: fresh,
        };
        if self.queue.try_send(sighting).is_ok() {
            kept.extend(remembered);
        }
    }
}

/// Start keeping reported calls in `store`. Each new pin is also wanted from
/// the modules that serve views, so a call's view is usually fetched before
/// any app asks for it, whether or not a provider ever writes for the call.
pub fn start(
    store: Arc<DisplayStore>,
    views: Option<Fetcher>,
) -> (CallPins, tokio::task::JoinHandle<()>) {
    let (queue, mut sightings) = mpsc::channel::<Sighting>(QUEUE);
    let task = tokio::spawn(async move {
        let mut wanted: HashSet<String> = HashSet::new();
        while let Some(first) = sightings.recv().await {
            let mut batch = vec![first];
            while let Ok(next) = sightings.try_recv() {
                batch.push(next);
            }
            let rows: Vec<(String, String, String, String)> = batch
                .iter()
                .flat_map(|sighting| {
                    sighting.calls.iter().map(|(minted_by, call_key, pin)| {
                        (
                            minted_by.clone(),
                            call_key.clone(),
                            sighting.session.clone(),
                            pin.clone(),
                        )
                    })
                })
                .collect();
            if let Some(views) = &views {
                for (_, _, _, pin) in &rows {
                    if wanted.len() >= REMEMBERED {
                        wanted.clear();
                    }
                    if wanted.insert(pin.clone()) {
                        views.want_pin(pin);
                    }
                }
            }
            let store = store.clone();
            let kept = tokio::task::spawn_blocking(move || store.keep_call_pins(&rows, now_ms()));
            match kept.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(%error, "cannot keep call pins"),
                Err(error) => tracing::warn!(%error, "keeping call pins stopped early"),
            }
        }
    });
    (
        CallPins {
            queue,
            kept: Mutex::new(HashSet::new()),
        },
        task,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortexkit_test_support::ScratchDir;

    fn call(key: &str, pin: &str) -> CallIdentity {
        CallIdentity {
            call_key: key.to_owned(),
            schema_pin: pin.to_owned(),
            carrier: None,
        }
    }

    /// A page's calls are kept with their runner and session, each new pin is
    /// wanted once from the modules that serve views, and a page built again
    /// costs nothing.
    #[tokio::test]
    async fn a_pages_calls_are_kept_and_their_pins_wanted_once() {
        let dir = ScratchDir::new("occipital-call-pins");
        let descriptor = crate::store::tests::descriptor(&dir);
        let store = Arc::new(DisplayStore::open(&descriptor, dir.path()).unwrap());
        let (queue, mut wanted) = mpsc::channel(16);
        let (pins, _task) = start(store.clone(), Some(Fetcher::from_queue(queue)));
        pins.seen(
            "thalamus-harness",
            "ses_1",
            vec![call("ck_1", "tp1:bash:00:1"), call("ck_2", "tp1:bash:00:1")],
        );
        pins.seen(
            "thalamus-harness",
            "ses_1",
            vec![call("ck_1", "tp1:bash:00:1")],
        );
        pins.seen("broca", "alfonso:bg_1", vec![call("ck_9", "tp1:read:00:1")]);
        for _ in 0..1_000 {
            if store.call_pin("broca", "ck_9").unwrap().is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        assert_eq!(
            store.call_pin("thalamus-harness", "ck_2").unwrap(),
            Some(("ses_1".into(), "tp1:bash:00:1".into()))
        );
        assert_eq!(
            store.call_pin("broca", "ck_9").unwrap(),
            Some(("alfonso:bg_1".into(), "tp1:read:00:1".into()))
        );
        let mut pins_wanted = Vec::new();
        while let Ok(want) = wanted.try_recv() {
            pins_wanted.push(want);
        }
        assert_eq!(
            pins_wanted,
            [
                super::super::views::Want::Pin("tp1:bash:00:1".into()),
                super::super::views::Want::Pin("tp1:read:00:1".into())
            ]
        );
    }

    /// A call already kept in one session is kept again when a page of another session
    /// carries it, as after its history moved, so the store follows it there.
    #[tokio::test]
    async fn a_call_seen_in_another_session_is_kept_again() {
        let dir = ScratchDir::new("occipital-call-pins-moved");
        let descriptor = crate::store::tests::descriptor(&dir);
        let store = Arc::new(DisplayStore::open(&descriptor, dir.path()).unwrap());
        let (pins, _task) = start(store.clone(), None);
        let kept_in = |session: &'static str| {
            let store = store.clone();
            async move {
                for _ in 0..1_000 {
                    if store.call_pin("thalamus-harness", "ck_1").unwrap()
                        == Some((session.into(), "tp1:bash:00:1".into()))
                    {
                        return true;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                false
            }
        };
        pins.seen(
            "thalamus-harness",
            "ses_1",
            vec![call("ck_1", "tp1:bash:00:1")],
        );
        assert!(kept_in("ses_1").await);
        pins.seen(
            "thalamus-harness",
            "ses_2",
            vec![call("ck_1", "tp1:bash:00:1")],
        );
        assert!(kept_in("ses_2").await);
    }
}
