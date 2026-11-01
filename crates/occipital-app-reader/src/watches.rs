//! Module-local capacity and route-lifetime bookkeeping for both app watches.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

pub const MAX_OPEN_WATCHES: usize = 32;

/// An attested route's channel and epoch, supplied by the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchRoute {
    pub channel: u32,
    pub epoch: u64,
}

#[derive(Default)]
pub struct WatchRegistry {
    state: Mutex<RegistryState>,
}

#[derive(Default)]
struct RegistryState {
    next_id: u64,
    open: HashMap<u64, OpenWatch>,
    stopping: bool,
}

struct OpenWatch {
    route: WatchRoute,
    end: watch::Sender<bool>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OpenRefusal {
    Capacity,
    Stopping,
}

impl WatchRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, RegistryState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn open(self: &Arc<Self>, route: WatchRoute) -> Result<WatchSlot, OpenRefusal> {
        let mut state = self.lock();
        if state.stopping {
            return Err(OpenRefusal::Stopping);
        }
        if state.open.len() >= MAX_OPEN_WATCHES {
            return Err(OpenRefusal::Capacity);
        }
        let id = state.next_id;
        state.next_id += 1;
        let (end, ended) = watch::channel(false);
        state.open.insert(id, OpenWatch { route, end });
        Ok(WatchSlot {
            registry: Arc::clone(self),
            id,
            ended,
        })
    }

    /// Signal StreamEnd without releasing capacity until the serving task drops its slot.
    pub fn end_route(&self, route: WatchRoute) {
        for watch in self.lock().open.values() {
            if watch.route == route {
                let _ = watch.end.send(true);
            }
        }
    }

    pub fn end_all(&self) {
        let mut state = self.lock();
        state.stopping = true;
        for watch in state.open.values() {
            let _ = watch.end.send(true);
        }
    }

    pub fn open_count(&self) -> usize {
        self.lock().open.len()
    }
}

/// Dropping a slot releases capacity regardless of how the watch ended.
pub struct WatchSlot {
    registry: Arc<WatchRegistry>,
    id: u64,
    ended: watch::Receiver<bool>,
}

impl WatchSlot {
    pub fn ended(&self) -> watch::Receiver<bool> {
        self.ended.clone()
    }
}

impl Drop for WatchSlot {
    fn drop(&mut self) {
        self.registry.lock().open.remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thirty_third_watch_is_refused_and_slots_free_when_watches_end() {
        let registry = Arc::new(WatchRegistry::default());
        let route = WatchRoute {
            channel: 1,
            epoch: 2,
        };
        let mut slots: Vec<_> = (0..32).map(|_| registry.open(route).unwrap()).collect();
        assert!(matches!(registry.open(route), Err(OpenRefusal::Capacity)));
        assert_eq!(registry.open_count(), 32);
        slots.pop();
        slots.push(registry.open(route).unwrap());
        registry.end_route(WatchRoute {
            channel: 1,
            epoch: 3,
        });
        assert!(slots.iter().all(|slot| !*slot.ended.borrow()));
        registry.end_route(route);
        assert!(slots.iter().all(|slot| *slot.ended.borrow()));
        drop(slots);
        assert_eq!(registry.open_count(), 0);
        assert!(registry.open(route).is_ok());
    }

    #[test]
    fn module_shutdown_ends_every_open_watch_and_refuses_new_ones() {
        let registry = Arc::new(WatchRegistry::default());
        let slots: Vec<_> = (0..2)
            .map(|channel| registry.open(WatchRoute { channel, epoch: 1 }).unwrap())
            .collect();
        registry.end_all();
        assert!(slots.iter().all(|slot| *slot.ended.borrow()));
        assert!(matches!(
            registry.open(WatchRoute {
                channel: 3,
                epoch: 1
            }),
            Err(OpenRefusal::Stopping)
        ));
    }
}
