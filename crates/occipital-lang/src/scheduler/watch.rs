use super::identity::{Identity, Reads};
use super::protocol;
use super::query::{Queries, Query};
use super::{Effect, Epoch, Timer};
use crate::{Decimal, Operation, Value, ValueKind};

const CREDIT_BYTES: u64 = 512 * 1024;

#[derive(Clone, Debug, Default)]
pub(super) struct Watch {
    pub dropped: u64,
    last: u64,
    pub(super) ack: bool,
    epoch: Option<Epoch>,
    token: u64,
    pending: Vec<(u64, Value)>,
    reserved: u64,
    notices: u32,
    applying: bool,
    terminal: Option<Option<String>>,
}

impl Watch {
    pub fn pending_frames(&self) -> impl Iterator<Item = &Value> {
        self.pending.iter().map(|(_, frame)| frame)
    }
    pub fn retire(&mut self) {
        self.pending.clear();
        self.reserved = 0;
        self.notices = 0;
        self.applying = false;
        self.epoch = None;
        self.ack = false;
        self.terminal = None;
    }
}

pub(super) fn reset(query: &mut Query) {
    query.watch = Some(Watch::default());
    query.value = Value::items(vec![]);
    query.status = "loading";
    query.error = None;
    query.stale = false;
    query.ready = false;
}

pub(super) fn restart(query: &mut Query, reads: &Reads, identity: &Identity) {
    if let Some(Operation::Watch {
        resume: false,
        replay: true,
        ..
    }) = reads.registry.0.get(&identity.operation)
    {
        let stale = query.stale;
        reset(query);
        query.stale = stale;
    }
    query.status = "loading";
    query.error = None;
}

pub(super) fn subscribe(
    query: &mut Query,
    reads: &Reads,
    identity: &Identity,
    site: String,
    epoch: Epoch,
    effects: &mut Vec<Effect>,
) {
    restart(query, reads, identity);
    // Only watch queries subscribe, and each holds its watch state.
    let Some(watch) = query.watch.as_mut() else {
        return;
    };
    watch.epoch = Some(epoch);
    watch.token = 0;
    let resume = matches!(
        reads.registry.0.get(&identity.operation),
        Some(Operation::Watch { resume: true, .. })
    );
    effects.push(Effect::Subscribe {
        site,
        identity: identity.clone(),
        epoch,
        after: (resume && watch.last > 0).then_some(watch.last),
    });
    effects.push(Effect::Credit {
        epoch,
        notices: 16,
        bytes: CREDIT_BYTES,
    });
}

impl Queries {
    pub fn watch_ack(
        &mut self,
        identity: &Identity,
        epoch: Epoch,
        after: u64,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some(query) = self
            .shared
            .get_mut(identity)
            .filter(|q| protocol::matches(q, epoch))
        else {
            return false;
        };
        let Some(watch) = query.watch.as_mut() else {
            return false;
        };
        if watch.ack {
            return false;
        }
        watch.ack = true;
        watch.last = after;
        query.status = "live";
        query.ready = true;
        query.answered = true;
        query.error = None;
        effects.push(Effect::Stop(Timer::Timeout(epoch)));
        true
    }

    pub fn watch_end(
        &mut self,
        identity: &Identity,
        epoch: Epoch,
        error: Option<String>,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some(query) = self
            .shared
            .get_mut(identity)
            .filter(|q| protocol::matches(q, epoch))
        else {
            return false;
        };
        let Some(watch) = query.watch.as_mut() else {
            return false;
        };
        // Terminal events must not flush notices between application ticks;
        // otherwise a busy stream could bypass the four-per-second cap (§8.5).
        if watch.applying {
            if watch.terminal.is_none() {
                watch.terminal = Some(error);
            }
            return false;
        }
        watch.retire();
        query.request = None;
        query.pending = None;
        query.answered = true;
        effects.push(Effect::Stop(Timer::Timeout(epoch)));
        if let Some(error) = error {
            effects.push(Effect::Cancel {
                site: self
                    .sites
                    .iter()
                    .find(|(_, s)| s.identity.as_ref() == Some(identity))
                    .map(|(n, _)| n.clone())
                    .unwrap_or_default(),
                identity: identity.clone(),
                epoch,
            });
            query.status = "error";
            query.error = Some(error);
            query.stale = true;
        } else {
            query.status = "ended";
            query.error = None;
            query.ready = true;
        }
        self.reconcile_apply(effects);
        true
    }

    pub fn watch_notice(
        &mut self,
        identity: &Identity,
        epoch: Epoch,
        token: u64,
        first: u64,
        bytes: &[u8],
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some(query) = self
            .shared
            .get_mut(identity)
            .filter(|q| q.watch.is_some() && protocol::matches(q, epoch))
        else {
            return false;
        };
        let Some(watch) = query.watch.as_ref() else {
            return false;
        };
        if watch.terminal.is_some() {
            return false;
        }
        if token <= watch.token {
            return false;
        }
        if !watch.ack {
            return self.watch_end(identity, epoch, Some("protocol error".into()), effects);
        }
        let parsed = protocol::decode(&self.accounts, bytes, 32 * 1024);
        if parsed.as_ref().is_err_and(|error| error == "budget") {
            Self::stop(query, identity, effects);
            query.status = "error";
            query.error = Some("too_large".into());
            query.stale = true;
            self.reconcile_apply(effects);
            return true;
        }
        let parsed = parsed.map_err(|error| {
            if error == "too_large" {
                "too_large".into()
            } else {
                "protocol error".into()
            }
        });
        let valid = parsed.and_then(|value| {
            let ValueKind::List(items) = value.kind() else {
                return Err("protocol error".into());
            };
            // A watch identity is only made for a registered watch operation,
            // and the registry is fixed for the session.
            let Some(Operation::Watch { item, .. }) =
                self.reads.registry.0.get(&identity.operation)
            else {
                return Err("protocol error".into());
            };
            if value.logical_size() > 128 * 1024
                || items.iter().any(|v| v.logical_size() > 8 * 1024)
            {
                return Err("too_large".into());
            }
            if items.iter().any(|v| !protocol::accepts(item, v)) {
                return Err("protocol error".into());
            }
            if !items.is_empty()
                && (first == 0 || first.checked_add(items.len() as u64 - 1).is_none())
            {
                return Err("protocol error".into());
            }
            Ok((
                value.clone(),
                (bytes.len() as u64).max(value.logical_size()),
            ))
        });
        let (frame, charge) = match valid {
            Ok(v) => v,
            Err(error) => return self.watch_end(identity, epoch, Some(error), effects),
        };
        let Some(watch) = query.watch.as_mut() else {
            return false;
        };
        if watch.notices == 16 || charge > CREDIT_BYTES - watch.reserved {
            return self.watch_end(identity, epoch, Some("too_large".into()), effects);
        }
        // Reservation tokens are monotonic per epoch, independently of item
        // sequences. A redelivery cannot release a reservation twice (§8.5).
        watch.token = token;
        watch.reserved += charge;
        watch.notices += 1;
        watch.pending.push((first, frame));
        if !watch.applying {
            watch.applying = true;
            if self.apply_timer.is_none() {
                let Some(generation) = self.apply_generation.checked_add(1) else {
                    self.retired = true;
                    self.cancel_all(effects);
                    return false;
                };
                self.apply_generation = generation;
                let timer = Timer::WatchApply(epoch, generation);
                self.apply_timer = Some(timer.clone());
                effects.push(Effect::Arm(timer, Decimal::from_scaled(25, 2)));
            }
        }
        false
    }

    pub fn watch_apply(&mut self, timer: &Timer, effects: &mut Vec<Effect>) -> bool {
        if self.apply_timer.as_ref() != Some(timer) {
            return false;
        }
        self.apply_timer = None;
        let epochs: Vec<_> = self
            .shared
            .values()
            .filter_map(|q| {
                q.watch
                    .as_ref()
                    .filter(|w| w.applying)
                    .and_then(|w| w.epoch)
            })
            .collect();
        let mut changed = false;
        for epoch in epochs {
            changed |= self.apply_watch(epoch, effects);
        }
        changed
    }

    pub fn reconcile_apply(&mut self, effects: &mut Vec<Effect>) {
        if !self
            .shared
            .values()
            .any(|q| q.watch.as_ref().is_some_and(|w| w.applying))
        {
            if let Some(timer) = self.apply_timer.take() {
                effects.push(Effect::Stop(timer));
            }
        }
    }

    fn apply_watch(&mut self, epoch: Epoch, effects: &mut Vec<Effect>) -> bool {
        let Some(query) = self.shared.values_mut().find(|q| {
            q.watch
                .as_ref()
                .is_some_and(|w| w.epoch == Some(epoch) && w.applying)
        }) else {
            return false;
        };
        let Some(watch) = query.watch.as_mut() else {
            return false;
        };
        // A watch query's value is always its list of items so far.
        let mut buffer = match query.value.kind() {
            ValueKind::List(buffer) => buffer.to_vec(),
            _ => Vec::new(),
        };
        for (first, frame) in watch.pending.drain(..) {
            // `watch_notice` only queues frames it checked are lists.
            let ValueKind::List(items) = frame.kind() else {
                continue;
            };
            query.stale = false;
            if items.is_empty() {
                continue;
            }
            let last = first + items.len() as u64 - 1;
            if last <= watch.last {
                continue;
            }
            if first > watch.last.saturating_add(1) {
                watch.dropped = watch.dropped.saturating_add(first - watch.last - 1);
            }
            let skip = watch
                .last
                .saturating_sub(first)
                .saturating_add(1)
                .min(items.len() as u64);
            let skip = if first > watch.last { 0 } else { skip as usize };
            buffer.extend(items.iter().skip(skip).cloned());
            watch.last = last;
        }
        let mut size = Value::items(vec![]).logical_size()
            + buffer.iter().map(Value::logical_size).sum::<u64>();
        let mut evicted = 0;
        while buffer.len() - evicted > 1000 || size > 256 * 1024 {
            size -= buffer[evicted].logical_size();
            evicted += 1;
        }
        buffer.drain(..evicted);
        watch.dropped = watch.dropped.saturating_add(evicted as u64);
        // The eviction above keeps the buffer within 256 KiB.
        query.value = Value::items(buffer);
        let terminal = watch.terminal.take();
        if terminal.is_none() {
            effects.push(Effect::Credit {
                epoch,
                notices: watch.notices,
                bytes: watch.reserved,
            });
        }
        watch.notices = 0;
        watch.reserved = 0;
        watch.applying = false;
        if let Some(error) = terminal {
            // The query applied above matches this epoch.
            let identity = self
                .shared
                .iter()
                .find(|(_, q)| protocol::matches(q, epoch))
                .map(|(identity, _)| identity.clone());
            if let Some(identity) = identity {
                self.watch_end(&identity, epoch, error, effects);
            }
        }
        true
    }
}
