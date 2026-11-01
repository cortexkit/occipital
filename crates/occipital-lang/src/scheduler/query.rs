use std::collections::{BTreeMap, BTreeSet};

use super::identity::{Identity, Reads};
use super::{Effect, Epoch, Timer};
use crate::eval::{Demand, SiteResolver};
use crate::{Decimal, Operation, ScalarString, SiteEnvelope, Value, ValueKind};

#[derive(Clone, Debug)]
pub(super) struct Query {
    pub value: Value,
    pub(super) default: Value,
    pub status: &'static str,
    pub(super) error: Option<String>,
    pub(super) stale: bool,
    pub(super) ready: bool,
    pub(super) answered: bool,
    errors: u32,
    pub request: Option<(String, Epoch)>,
    pub timer: Option<Timer>,
    pub issue: bool,
    pub(super) pending: Option<std::sync::Arc<crate::accounts::Reservation>>,
    pub(super) watch: Option<super::watch::Watch>,
    pub(super) page: Option<super::paged::Paged>,
}

impl Query {
    fn new(default: Value, previous: Option<Value>) -> Self {
        Self {
            value: previous.clone().unwrap_or_else(|| default.clone()),
            default,
            status: "loading",
            error: None,
            stale: previous.is_some(),
            ready: false,
            answered: false,
            errors: 0,
            request: None,
            timer: None,
            issue: true,
            pending: None,
            watch: None,
            page: None,
        }
    }

    pub(super) fn envelope(&self, name: &str) -> Value {
        let mut fields = vec![
            (ScalarString::new("value"), self.value.clone()),
            (ScalarString::new("status"), Value::text(self.status)),
            (
                ScalarString::new("error"),
                self.error
                    .as_deref()
                    .map(Value::text)
                    .unwrap_or_else(Value::null),
            ),
            (ScalarString::new("stale"), Value::boolean(self.stale)),
        ];
        if let Some(watch) = &self.watch {
            fields.push((
                ScalarString::new("dropped"),
                Value::number(Decimal::from_u64(watch.dropped)),
            ));
        }
        if let Some(page) = &self.page {
            fields[0].0 = ScalarString::new("items");
            fields.extend(page.fields());
        }
        Value::site_envelope(SiteEnvelope {
            name: name.into(),
            members: crate::Record::members(fields),
        })
    }

    fn finish(
        &mut self,
        answer: Result<Value, String>,
        identity: &Identity,
        epoch: Epoch,
        effects: &mut Vec<Effect>,
    ) {
        self.request = None;
        self.pending = None;
        self.answered = true;
        effects.push(Effect::Stop(Timer::Timeout(epoch)));
        match answer {
            Ok(value) => {
                self.value = value;
                self.status = "ready";
                self.error = None;
                self.stale = false;
                self.ready = true;
                self.errors = 0;
            }
            Err(error) => {
                self.status = "error";
                self.error = Some(error);
                self.stale = self.ready;
                if !self.ready {
                    self.value = self.default.clone();
                }
                self.errors = self.errors.saturating_add(1);
            }
        }
        if let Some(base) = identity.refresh {
            let multiplier = 1u64.checked_shl(self.errors.min(63)).unwrap_or(u64::MAX);
            let cap = base.max(Decimal::from_i64(300));
            let delay = base
                .checked_mul(Decimal::from_u64(multiplier))
                .unwrap_or(cap)
                .min(cap);
            let timer = Timer::Refresh(Box::new(identity.clone()), epoch);
            self.timer = Some(timer.clone());
            effects.push(Effect::Arm(timer, delay));
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Site {
    pub identity: Option<Identity>,
    pub local: Query,
    pub demanded: bool,
}

pub(super) struct Queries {
    pub accounts: crate::accounts::Accounts,
    pub reads: Reads,
    pub sites: BTreeMap<String, Site>,
    pub shared: BTreeMap<Identity, Query>,
    pub order: BTreeMap<String, usize>,
    pub epoch: u64,
    counter: u64,
    pub retired: bool,
    pub active: bool,
    visible: bool,
    foreground: bool,
    connected: bool,
    hidden: Option<Timer>,
    offline: Option<Timer>,
    policy_counter: u64,
    refreshes: BTreeSet<String>,
    more: BTreeSet<String>,
    intent_seen: BTreeSet<Identity>,
    deferred: Vec<Effect>,
    pub(super) apply_timer: Option<Timer>,
    pub(super) apply_generation: u64,
}

impl Queries {
    pub fn retained_buffers(&self) -> impl Iterator<Item = (&Identity, &'static str, &Value)> {
        self.shared.iter().flat_map(|(identity, query)| {
            let mut roots = Vec::new();
            if let Some(watch) = &query.watch {
                roots.push((identity, "watch_buffer", &query.value));
                roots.extend(
                    watch
                        .pending_frames()
                        .map(|value| (identity, "pending_notice", value)),
                );
            }
            if query.page.is_some() {
                roots.push((identity, "retained_pages", &query.value));
            }
            roots
        })
    }

    pub fn reload(&mut self, epoch: u64) -> Self {
        let mut next = Self::new(self.reads.clone(), self.order.clone(), epoch);
        next.accounts = self.accounts.fresh_local();
        // Host visibility and connection lifetimes outlive a program reload;
        // loading must not restart the hidden grace or offline deadline (§8.7).
        next.active = self.active;
        next.visible = self.visible;
        next.foreground = self.foreground;
        next.connected = self.connected;
        next.hidden = self.hidden.take();
        next.offline = self.offline.take();
        next.policy_counter = self.policy_counter;
        next
    }
    pub fn new(reads: Reads, order: BTreeMap<String, usize>, epoch: u64) -> Self {
        Self {
            accounts: crate::accounts::Accounts::interface(),
            reads,
            sites: BTreeMap::new(),
            shared: BTreeMap::new(),
            order,
            epoch,
            counter: 0,
            retired: false,
            active: true,
            visible: true,
            foreground: true,
            connected: true,
            hidden: None,
            offline: None,
            policy_counter: 0,
            refreshes: BTreeSet::new(),
            more: BTreeSet::new(),
            intent_seen: BTreeSet::new(),
            deferred: vec![],
            apply_timer: None,
            apply_generation: 0,
        }
    }

    pub fn begin(&mut self) {
        self.intent_seen.clear();
        for site in self.sites.values_mut() {
            site.demanded = false;
        }
    }

    pub fn intents(&mut self, refresh: &BTreeSet<String>, more: &BTreeSet<String>) {
        self.refreshes = refresh.clone();
        self.more = more.clone();
    }

    pub fn settled(&self) -> bool {
        self.sites
            .values()
            .filter(|site| site.demanded)
            .all(|site| {
                site.identity
                    .as_ref()
                    .and_then(|i| self.shared.get(i))
                    .unwrap_or(&site.local)
                    .status
                    != "loading"
            })
    }

    pub(super) fn stop(query: &mut Query, identity: &Identity, effects: &mut Vec<Effect>) {
        query.pending = None;
        if let Some((site, epoch)) = query.request.take() {
            effects.push(Effect::Cancel {
                site,
                identity: identity.clone(),
                epoch,
            });
            effects.push(Effect::Stop(Timer::Timeout(epoch)));
        }
        if let Some(timer) = query.timer.take() {
            effects.push(Effect::Stop(timer));
        }
        if let Some(watch) = &mut query.watch {
            watch.retire();
        }
        if let Some(page) = &mut query.page {
            page.loading_more = false;
        }
    }

    pub fn cancel_all(&mut self, effects: &mut Vec<Effect>) {
        effects.append(&mut self.deferred);
        for (identity, query) in &mut self.shared {
            Self::stop(query, identity, effects);
        }
        self.shared.clear();
        self.sites.clear();
        self.reconcile_apply(effects);
    }

    pub fn availability(&mut self, active: bool, effects: &mut Vec<Effect>) {
        if self.active == active {
            return;
        }
        self.active = active;
        for (identity, query) in &mut self.shared {
            Self::stop(query, identity, effects);
            query.stale = true;
            query.issue = match identity.kind {
                "watch" => {
                    if query.status == "ended" {
                        false
                    } else {
                        if active {
                            super::watch::restart(query, &self.reads, identity);
                        }
                        true
                    }
                }
                "paged" => query.page.as_ref().is_some_and(|p| p.pending),
                _ => true,
            };
        }
        self.reconcile_apply(effects);
    }

    pub fn visibility(&mut self, visible: bool, effects: &mut Vec<Effect>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if let Some(timer) = self.hidden.take() {
            effects.push(Effect::Stop(timer));
        }
        if visible {
            self.availability(self.foreground && self.connected, effects);
        } else {
            self.policy_counter = self.policy_counter.saturating_add(1);
            let timer = Timer::Hidden(Epoch {
                instance: self.epoch,
                request: self.policy_counter,
            });
            self.hidden = Some(timer.clone());
            effects.push(Effect::Arm(timer, Decimal::from_i64(2)));
        }
    }

    pub fn foreground(&mut self, foreground: bool, effects: &mut Vec<Effect>) {
        self.foreground = foreground;
        self.availability(
            foreground && self.connected && (self.visible || self.hidden.is_some()),
            effects,
        );
    }

    pub fn connection(&mut self, connected: bool, effects: &mut Vec<Effect>) {
        if self.connected == connected {
            return;
        }
        self.connected = connected;
        if let Some(timer) = self.offline.take() {
            effects.push(Effect::Stop(timer));
        }
        if !connected {
            self.policy_counter = self.policy_counter.saturating_add(1);
            let timer = Timer::Offline(Epoch {
                instance: self.epoch,
                request: self.policy_counter,
            });
            self.offline = Some(timer.clone());
            effects.push(Effect::Arm(timer, Decimal::from_i64(30)));
        }
        self.availability(
            connected && self.foreground && (self.visible || self.hidden.is_some()),
            effects,
        );
    }

    pub fn finalize(&mut self, refreshes: &BTreeSet<String>, effects: &mut Vec<Effect>) {
        self.finalize_more(refreshes, &BTreeSet::new(), effects);
    }

    pub fn finalize_more(
        &mut self,
        refreshes: &BTreeSet<String>,
        more: &BTreeSet<String>,
        effects: &mut Vec<Effect>,
    ) {
        effects.append(&mut self.deferred);
        self.sites.retain(|_, site| {
            site.demanded || site.identity.as_ref().is_some_and(|i| i.kind == "watch")
        });
        let identities: BTreeSet<_> = self
            .sites
            .values()
            .filter(|site| site.demanded)
            .filter_map(|site| site.identity.clone())
            .collect();
        self.shared.retain(|identity, query| {
            if identities.contains(identity) {
                true
            } else {
                Self::stop(query, identity, effects);
                if identity.kind == "watch"
                    && self
                        .sites
                        .values()
                        .any(|s| s.identity.as_ref() == Some(identity))
                {
                    query.stale = true;
                    query.issue = query.status != "ended";
                    true
                } else {
                    false
                }
            }
        });
        let refresh: BTreeSet<_> = refreshes
            .iter()
            .filter_map(|name| self.sites.get(name)?.identity.clone())
            .collect();
        for identity in refresh {
            if self.intent_seen.contains(&identity) {
                continue;
            }
            if let Some(query) = self.shared.get_mut(&identity) {
                Self::stop(query, &identity, effects);
                query.errors = 0;
                if identity.kind == "watch" {
                    super::watch::reset(query);
                }
                if identity.kind == "paged" {
                    super::paged::refresh(query);
                }
                query.issue = true;
            }
        }
        for name in more {
            if let Some(query) = self
                .sites
                .get(name)
                .filter(|s| s.demanded)
                .and_then(|s| s.identity.as_ref())
                .and_then(|i| self.shared.get_mut(i))
            {
                super::paged::load_more(query);
            }
        }
        self.refreshes.clear();
        self.more.clear();
        self.intent_seen.clear();
        self.reconcile_apply(effects);
        self.dispatch(effects);
    }

    fn dispatch(&mut self, effects: &mut Vec<Effect>) {
        if !self.active || self.retired {
            return;
        }
        let mut in_flight = self
            .shared
            .values()
            .filter(|q| q.watch.is_none() && q.request.is_some())
            .count();
        let mut queue: Vec<_> = self
            .shared
            .iter()
            .filter(|(_, q)| q.issue)
            .filter_map(|(identity, _)| {
                let (name, _) = self
                    .sites
                    .iter()
                    .filter(|(_, site)| site.demanded && site.identity.as_ref() == Some(identity))
                    .min_by_key(|(name, _)| self.order[*name])?;
                Some((self.order[name], name.clone(), identity.clone()))
            })
            .collect();
        queue.sort_by_key(|entry| entry.0);
        for (_, site, identity) in queue {
            if in_flight == 4 && identity.kind != "watch" {
                continue;
            }
            let Some(counter) = self.counter.checked_add(1) else {
                self.retired = true;
                self.cancel_all(effects);
                break;
            };
            self.counter = counter;
            let epoch = Epoch {
                instance: self.epoch,
                request: counter,
            };
            // The queue was built from `shared`'s own keys just above.
            let Some(query) = self.shared.get_mut(&identity) else {
                continue;
            };
            query.issue = false;
            if self
                .accounts
                .begin(crate::accounts::UnitKind::AnswerConversion)
                .is_err()
            {
                query.finish(Err("too_large".into()), &identity, epoch, effects);
                continue;
            }
            if identity.authority.serving_computer != identity.authority.verified_holder
                || identity.authority.computer != identity.authority.serving_computer
            {
                query.finish(Err("E_SITE_ARGS".into()), &identity, epoch, effects);
                continue;
            }
            query.request = Some((site.clone(), epoch));
            let bytes = match identity.kind {
                "watch" => 512 * 1024,
                "paged" => 256 * 1024,
                _ => 1024 * 1024,
            };
            match self.accounts.retain(crate::RetainedAllocation {
                requested: bytes,
                unused_capacity: 0,
            }) {
                Ok(reservation) => query.pending = Some(std::sync::Arc::new(reservation)),
                Err(_) => {
                    query.finish(Err("too_large".into()), &identity, epoch, effects);
                    continue;
                }
            }
            if identity.kind == "watch" {
                super::watch::subscribe(query, &self.reads, &identity, site, epoch, effects);
            } else if identity.kind == "paged" {
                super::paged::issue(query, &identity, site, epoch, effects);
                in_flight += 1;
            } else {
                effects.push(Effect::Issue {
                    site,
                    identity,
                    epoch,
                });
                in_flight += 1;
            }
            effects.push(Effect::Arm(Timer::Timeout(epoch), Decimal::from_i64(30)));
        }
    }

    pub fn answer(
        &mut self,
        site: &str,
        identity: &Identity,
        epoch: Epoch,
        bytes: Result<&[u8], &str>,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some(query) = self.shared.get_mut(identity) else {
            return false;
        };
        if query
            .request
            .as_ref()
            .is_none_or(|(issued, e)| issued != site || *e != epoch)
        {
            return false;
        }
        let answer = bytes.map_err(str::to_owned).and_then(|bytes| {
            if bytes.len() > 1_048_576 {
                return Err("answer too large".into());
            }
            let span = crate::SourceSpan {
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            };
            let parsed = crate::data::prepare_accounted(
                &self.accounts,
                crate::accounts::UnitKind::AnswerConversion,
                bytes,
                crate::data::Format::Json,
                &Default::default(),
                span,
            )
            .map_err(|_| "too_large".to_owned())?;
            if let ValueKind::Error(error) = parsed.value.kind() {
                return Err(if error.code.starts_with("E_DATA_LIMIT") {
                    "answer too large".into()
                } else {
                    error.code.to_string()
                });
            }
            let Some(Operation::Query { result, .. }) =
                self.reads.registry.0.get(&identity.operation)
            else {
                return Err("E_SITE_ARGS".into());
            };
            let json =
                serde_json::from_str(&parsed.value.canonical_json().map_err(|_| "E_SITE_ARGS")?)
                    .map_err(|_| "E_SITE_ARGS")?;
            if !result.accepts(&json) {
                return Err("E_SITE_ARGS".into());
            }
            Ok(parsed.value)
        });
        query.finish(answer, identity, epoch, effects);
        true
    }

    pub fn timer(&mut self, timer: &Timer, effects: &mut Vec<Effect>) -> bool {
        match timer {
            Timer::WatchApply(_, _) => self.watch_apply(timer, effects),
            Timer::Hidden(_) if self.hidden.as_ref() == Some(timer) => {
                self.hidden = None;
                self.availability(false, effects);
                true
            }
            Timer::Offline(_) if self.offline.as_ref() == Some(timer) => {
                self.offline = None;
                for query in self.shared.values_mut().filter(|q| !q.answered) {
                    query.status = "error";
                    query.error = Some("offline".into());
                }
                true
            }
            Timer::Timeout(epoch) => {
                let found = self.shared.iter().find_map(|(i, q)| {
                    let (site, _) = q.request.as_ref().filter(|(_, e)| e == epoch)?;
                    q.watch
                        .as_ref()
                        .is_none_or(|w| !w.ack)
                        .then(|| (i.clone(), site.clone()))
                });
                if let Some((identity, site)) = found {
                    match identity.kind {
                        "watch" => {
                            self.watch_end(&identity, *epoch, Some("timed out".into()), effects)
                        }
                        "paged" => {
                            // A paged query always has its page state.
                            let cursor = self
                                .shared
                                .get(&identity)
                                .and_then(|query| query.page.as_ref())
                                .and_then(|page| page.cursor.clone());
                            self.page_answer(
                                &identity,
                                *epoch,
                                cursor.as_deref(),
                                Err("timed out"),
                                effects,
                            )
                        }
                        _ => self.answer(&site, &identity, *epoch, Err("timed out"), effects),
                    }
                } else {
                    false
                }
            }
            Timer::Refresh(identity, _) => {
                let Some(query) = self.shared.get_mut(identity.as_ref()) else {
                    return false;
                };
                if query.timer.as_ref() != Some(timer) {
                    return false;
                }
                query.timer = None;
                query.issue = true;
                true
            }
            _ => false,
        }
    }
}

impl SiteResolver for Queries {
    fn resolve(&mut self, demand: &Demand) -> Value {
        let name = demand.site.to_string();
        let previous = self.sites.get(&name).and_then(|site| {
            site.identity
                .as_ref()
                .and_then(|i| self.shared.get(i))
                .map(|q| q.value.clone())
                .or_else(|| site.local.stale.then(|| site.local.value.clone()))
        });
        let default = if demand.constructor.as_ref() == "Query" {
            demand.arguments.get(2).cloned().unwrap_or_else(Value::null)
        } else {
            Value::items(vec![])
        };
        let mut local = Query::new(default.clone(), previous);
        let dependencies: Vec<_> = demand
            .dependencies
            .iter()
            .filter_map(|v| {
                let ValueKind::SiteEnvelope(envelope) = v.kind() else {
                    return None;
                };
                Some(&envelope.members)
            })
            .collect();
        let dependency = dependencies.iter().find(|members| matches!(members.get("status").kind(), ValueKind::String(s) if s.as_str() == "error")).or_else(|| dependencies.iter().find(|members| !matches!(members.get("status").kind(), ValueKind::String(s) if matches!(s.as_str(), "live" | "ended") || (s.as_str() == "ready" && !matches!(members.get("stale").kind(), ValueKind::Boolean(true))))));
        let identity = if let Some(dependency) = dependency {
            if matches!(dependency.get("status").kind(), ValueKind::String(s) if s.as_str() == "error")
            {
                local.status = "error";
                local.error = Some("dependency failed".into());
            }
            None
        } else if let Some(error) = demand.arguments.get(1).and_then(argument_error) {
            local.status = "error";
            local.error = Some(error);
            None
        } else {
            match self
                .reads
                .identity_for(&demand.constructor, &demand.arguments)
            {
                Ok(identity) => Some(identity),
                _ => {
                    local.status = "error";
                    local.error = Some("E_SITE_ARGS".into());
                    None
                }
            }
        };
        if demand.constructor.as_ref() == "Watch" {
            local.value = default.clone();
            local.watch = Some(super::watch::Watch::default());
            local.stale = false;
        }
        if demand.constructor.as_ref() == "PagedList" {
            local.value = default.clone();
            local.page = Some(super::paged::Paged::default());
            local.stale = false;
        }
        let envelope = if let Some(identity) = &identity {
            let query = self
                .shared
                .entry(identity.clone())
                .or_insert_with(|| local.clone());
            if (self.refreshes.contains(&name) || self.more.contains(&name))
                && self.intent_seen.insert(identity.clone())
            {
                if self.refreshes.contains(&name) {
                    Self::stop(query, identity, &mut self.deferred);
                    query.errors = 0;
                    if identity.kind == "watch" {
                        super::watch::reset(query);
                    }
                    if identity.kind == "paged" {
                        super::paged::refresh(query);
                    }
                    query.issue = true;
                } else {
                    super::paged::load_more(query);
                }
            }
            if query.issue && self.active {
                let loading_more = query.ready && query.status != "loading";
                if let Some(page) = query.page.as_mut().filter(|p| p.pending) {
                    page.loading_more = loading_more;
                }
            }
            query.envelope(&name)
        } else {
            local.envelope(&name)
        };
        self.sites.insert(
            name,
            Site {
                identity,
                local,
                demanded: true,
            },
        );
        envelope
    }
}

fn argument_error(value: &Value) -> Option<String> {
    match value.kind() {
        ValueKind::Error(error) => Some(error.code.to_string()),
        ValueKind::List(items) => items.iter().find_map(argument_error),
        ValueKind::Record(record) => record.values().find_map(argument_error),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_counter_wrap_retires_the_instance_and_cancels_instead_of_reusing() {
        let reads = super::super::support::reads();
        let mut queries = Queries::new(reads, [("q".into(), 0)].into(), 42);
        let demand = Demand {
            site: "q".into(),
            constructor: "Query".into(),
            arguments: vec![
                Value::string("read").unwrap(),
                Value::record(vec![]).unwrap(),
                Value::null(),
            ],
            dependencies: vec![],
        };
        queries.resolve(&demand);
        let mut effects = vec![];
        queries.finalize(&BTreeSet::new(), &mut effects);
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::Issue { epoch, .. } if epoch.request == 1)));
        queries.counter = u64::MAX;
        effects.clear();
        queries.finalize(&["q".into()].into(), &mut effects);
        assert!(queries.retired);
        assert!(queries.shared.is_empty() && queries.sites.is_empty());
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::Cancel { epoch, .. } if epoch.request == 1)));
        assert!(!effects.iter().any(|e| matches!(e, Effect::Issue { .. })));
    }
}
