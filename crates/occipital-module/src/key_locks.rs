//! Serialize work on one key without making unrelated requests wait for it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

#[derive(Default)]
pub(crate) struct KeyLocks(Mutex<HashMap<String, Weak<AsyncMutex<()>>>>);

impl KeyLocks {
    pub(crate) async fn lock(&self, key: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.0.lock().unwrap_or_else(|p| p.into_inner());
            locks.retain(|_, lock| lock.strong_count() != 0);
            let entry = locks.entry(key.to_owned()).or_default();
            entry.upgrade().unwrap_or_else(|| {
                let lock = Arc::new(AsyncMutex::new(()));
                *entry = Arc::downgrade(&lock);
                lock
            })
        };
        lock.lock_owned().await
    }
}
