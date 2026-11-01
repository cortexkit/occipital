//! Keeping each call's pin as its runner stored it, while pages are built.
//!
//! An app asks for a call's records by its call key, and the join needs the
//! call's pin from the runner (tool-display §6), never from the app or the
//! write. The app holds a call key only because a page carried it, and every
//! page is built here from the runner's read, so building the page is where
//! the pin is learned. It's kept in the display store, since the app may ask
//! about a chat it cached long before this process started.
//!
//! Page building never waits on this: sightings go to a task through a
//! bounded queue, and one that doesn't fit is dropped. A dropped call is kept
//! the next time a page carrying it is built.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::mpsc;

use super::store::{now_ms, DisplayStore};
use super::views::Fetcher;

/// How many pages' sightings wait to be kept before more are dropped.
const QUEUE: usize = 256;

/// How many calls are remembered as already kept, so a page read again and
/// again costs no store write. Past this the memory starts over.
const REMEMBERED: usize = 100_000;

/// The calls one built page carries: its runner and session, and each call's
/// key and pin.
struct Sighting {
    runner: String,
    session: String,
    calls: Vec<(String, String)>,
}

/// Where page building reports calls, once the display store is open.
pub type CallPinSlot = Arc<OnceLock<CallPins>>;

pub struct CallPins {
    queue: mpsc::Sender<Sighting>,
    kept: Mutex<HashSet<(String, String)>>,
}

impl CallPins {
    /// Report the calls a page of `runner`'s `session` carries, each as
    /// `(call key, pin)`. Calls already kept are skipped; never waits.
    pub fn seen(&self, runner: &str, session: &str, calls: Vec<(String, String)>) {
        let Ok(mut kept) = self.kept.lock() else {
            return;
        };
        let fresh: Vec<(String, String)> = calls
            .into_iter()
            .filter(|(call_key, _)| !kept.contains(&(runner.to_owned(), call_key.clone())))
            .collect();
        if fresh.is_empty() {
            return;
        }
        if kept.len() + fresh.len() > REMEMBERED {
            kept.clear();
        }
        let sighting = Sighting {
            runner: runner.to_owned(),
            session: session.to_owned(),
            calls: fresh,
        };
        // Remembered only once queued, so a dropped sighting is reported again.
        let calls: Vec<(String, String)> = sighting
            .calls
            .iter()
            .map(|(call_key, _)| (runner.to_owned(), call_key.clone()))
            .collect();
        if self.queue.try_send(sighting).is_ok() {
            kept.extend(calls);
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
                    sighting.calls.iter().map(|(call_key, pin)| {
                        (
                            sighting.runner.clone(),
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

    fn call(key: &str, pin: &str) -> (String, String) {
        (key.to_owned(), pin.to_owned())
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
}
