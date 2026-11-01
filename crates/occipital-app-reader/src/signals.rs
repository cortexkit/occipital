//! Per-session event wakes for transcript readers.

use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::watch;

/// Per-session change signals for watches that wait on events instead of
/// polling. A watch subscribes under a session id; the code that observes an
/// event for that session calls [`SessionSignals::notify`]. Only sessions some
/// receiver is subscribed to need an entry. Entries without receivers are pruned
/// on a later subscription, or when that session is notified.
#[derive(Debug, Default)]
pub struct SessionSignals {
    senders: Mutex<HashMap<String, watch::Sender<u64>>>,
}

impl SessionSignals {
    /// A receiver that wakes after the next `notify` for `session_id`.
    pub fn subscribe(&self, session_id: &str) -> watch::Receiver<u64> {
        let mut senders = self.senders.lock().expect("session signals poisoned");
        senders.retain(|_, sender| sender.receiver_count() > 0);
        senders
            .entry(session_id.to_owned())
            .or_insert_with(|| watch::channel(0).0)
            .subscribe()
    }

    /// Signal receivers under `session_id`; this call may wait for the map mutex.
    pub fn notify(&self, session_id: &str) {
        let mut senders = self.senders.lock().expect("session signals poisoned");
        let Some(sender) = senders.get(session_id) else {
            return;
        };
        if sender.receiver_count() == 0 {
            senders.remove(session_id);
            return;
        }
        sender.send_modify(|count| *count = count.wrapping_add(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_signals_wake_only_their_own_session_and_drop_unwatched_ones() {
        let signals = SessionSignals::default();
        let a = signals.subscribe("a");
        let mut b = signals.subscribe("b");
        signals.notify("a");
        assert!(a.has_changed().unwrap());
        assert!(!b.has_changed().unwrap());
        b.borrow_and_update();
        drop(a);
        signals.notify("a");
        assert!(!signals.senders.lock().unwrap().contains_key("a"));
        signals.notify("unwatched");
        assert!(!signals.senders.lock().unwrap().contains_key("unwatched"));
    }
}
