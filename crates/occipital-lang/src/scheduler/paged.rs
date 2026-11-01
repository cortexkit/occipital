use std::collections::BTreeSet;

use super::identity::Identity;
use super::protocol;
use super::query::{Queries, Query};
use super::{Effect, Epoch, Timer};
use crate::{Decimal, Operation, ScalarString, Value, ValueKind};

#[derive(Clone, Debug)]
pub(super) struct Paged {
    pub cursor: Option<String>,
    next: Option<String>,
    seen: BTreeSet<String>,
    keys: BTreeSet<String>,
    pages: u32,
    done: bool,
    capped: bool,
    pub loading_more: bool,
    pub pending: bool,
    first: bool,
    invalid: bool,
    total: Value,
    progress: bool,
    token: Option<u64>,
}

impl Default for Paged {
    fn default() -> Self {
        Self {
            cursor: None,
            next: None,
            seen: BTreeSet::new(),
            keys: BTreeSet::new(),
            pages: 0,
            done: false,
            capped: false,
            loading_more: false,
            pending: true,
            first: true,
            invalid: false,
            total: Value::null(),
            progress: true,
            token: None,
        }
    }
}

impl Paged {
    pub fn fields(&self) -> Vec<(ScalarString, Value)> {
        vec![
            (ScalarString::new("done"), Value::boolean(self.done)),
            (ScalarString::new("total"), self.total.clone()),
            (
                ScalarString::new("loadingMore"),
                Value::boolean(self.loading_more),
            ),
            (ScalarString::new("capped"), Value::boolean(self.capped)),
        ]
    }
}

pub(super) fn refresh(query: &mut Query) {
    // Preserve the last-consumed sentinel-entry token across Refresh. The host
    // increases this counter when the end sentinel enters the viewport (§8.6).
    let token = query.page.as_ref().and_then(|p| p.token);
    query.page = Some(Paged {
        token,
        ..Paged::default()
    });
    query.status = "loading";
    query.error = None;
    query.stale = true;
}

pub(super) fn load_more(query: &mut Query) {
    let Some(page) = &mut query.page else { return };
    if query.request.is_some() || query.issue || page.done || page.capped || page.invalid {
        return;
    }
    if query.status == "error" {
        page.pending = true;
    } else {
        page.cursor = page.next.clone();
        page.pending = true;
    }
    query.issue = true;
}

pub(super) fn issue(
    query: &mut Query,
    identity: &Identity,
    site: String,
    epoch: Epoch,
    effects: &mut Vec<Effect>,
) {
    // A paged identity always carries its page size, and its query always
    // has page state; without either there is no page request to make.
    let (Some(page), Some(limit)) = (query.page.as_mut(), identity.page_size) else {
        return;
    };
    page.loading_more = !page.first;
    page.pending = true;
    effects.push(Effect::Page {
        site,
        identity: identity.clone(),
        epoch,
        cursor: page.cursor.clone(),
        limit,
    });
}

impl Queries {
    pub fn auto_more(&mut self, site: &str, token: u64) -> bool {
        let Some(query) = self
            .sites
            .get(site)
            .and_then(|s| s.identity.as_ref())
            .and_then(|i| self.shared.get_mut(i))
        else {
            return false;
        };
        let Some(page) = &mut query.page else {
            return false;
        };
        if page.token.is_some_and(|old| token <= old) {
            return false;
        }
        page.token = Some(token);
        query.status == "ready" && page.progress && !page.pending && !page.done && !page.capped
    }

    pub fn page_answer(
        &mut self,
        identity: &Identity,
        epoch: Epoch,
        cursor: Option<&str>,
        bytes: Result<&[u8], &str>,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some(query) = self
            .shared
            .get_mut(identity)
            .filter(|q| protocol::matches(q, epoch))
        else {
            return false;
        };
        let Some(page) = query.page.as_mut() else {
            return false;
        };
        if page.cursor.as_deref() != cursor {
            return false;
        }
        query.request = None;
        query.pending = None;
        query.answered = true;
        effects.push(Effect::Stop(Timer::Timeout(epoch)));
        page.pending = false;
        page.loading_more = false;
        let mut invalid_cursor = false;
        let result = bytes.map_err(str::to_owned).and_then(|bytes| {
            let value = protocol::decode(&self.accounts, bytes, 256 * 1024)
                .map_err(|e| if e == "budget" { "too_large".to_owned() } else { "invalid_page".to_owned() })?;
            // A paged identity is only made for a registered paged operation,
            // and the registry is fixed for the session. The query path answers
            // the same impossible mismatch with `E_SITE_ARGS`.
            let Some(Operation::Paged { page: schema, item_key, .. }) = self.reads.registry.0.get(&identity.operation) else { return Err("E_SITE_ARGS".into()) };
            let ValueKind::Record(record) = value.kind() else { return Err("invalid_page".into()) };
            let ValueKind::List(items) = record.get("items").kind().clone() else { return Err("invalid_page".into()) };
            let next = match record.get("next").kind() {
                ValueKind::Null if record.keys().any(|k| k == "next") => None,
                ValueKind::String(s) if s.as_str().len() <= 1024 => Some(s.as_str().to_owned()),
                _ => { invalid_cursor = true; return Err("invalid_page".into()); },
            };
            let total = if record.keys().any(|k| k == "total") { record.get("total") } else { Value::null() };
            if !matches!(total.kind(), ValueKind::Null) && !matches!(total.kind(), ValueKind::Number(n) if *n >= Decimal::from_i64(0) && !n.to_string().contains('.')) { return Err("invalid_page".into()); }
            if next.as_ref().is_some_and(|s| page.seen.contains(s) || page.cursor.as_ref() == Some(s)) { invalid_cursor = true; return Err("invalid_page".into()); }
            if !protocol::accepts(schema, &value) { return Err("invalid_page".into()); }
            if value.logical_size() > 1024 * 1024 || items.iter().any(|v| v.logical_size() > 64 * 1024) { return Err("invalid_page".into()); }
            let mut keys = if page.first { BTreeSet::new() } else { page.keys.clone() };
            let mut unique = Vec::new();
            for item in items.iter().cloned() {
                if let Some(key) = item_key {
                    let ValueKind::Record(record) = item.kind() else { return Err("invalid_page".into()) };
                    if !record.keys().any(|k| k == key) { return Err("invalid_page".into()); }
                    let key = record.get(key).canonical_json().map_err(|_| "invalid_page".to_owned())?;
                    if !keys.insert(key) { continue; }
                }
                // Equal unkeyed rows can represent distinct events. Only the
                // paged operation's configured item key authorizes deduplication.
                unique.push(item);
            }
            Ok((unique, keys, next, total))
        });
        match result {
            Err(error) => {
                page.invalid = invalid_cursor;
                page.progress = false;
                query.status = "error";
                query.error = Some(error);
                query.stale |= query.ready;
            }
            Ok((items, keys, next, total)) => {
                let mut retained = if page.first {
                    vec![]
                } else {
                    // A paged query's value is always the list of items so far.
                    match query.value.kind() {
                        ValueKind::List(v) => v.to_vec(),
                        _ => vec![],
                    }
                };
                let progress = !items.is_empty();
                retained.extend(items);
                let count = retained.len();
                // The size is checked against the 4 MiB cap just below.
                let candidate = Value::items(retained);
                // The post-apply page cap and LoadMore's capped guard prevent
                // a twenty-first page request. Only item/byte limits can be
                // exceeded by the pending append (§8.6).
                if count > 2000 || candidate.logical_size() > 4 * 1024 * 1024 {
                    page.capped = true;
                    page.progress = false;
                    query.status = "ready";
                    query.error = None;
                } else {
                    page.pages += 1;
                    page.keys = keys;
                    if let Some(cursor) = &page.cursor {
                        page.seen.insert(cursor.clone());
                    }
                    page.capped = page.pages == 20
                        || count == 2000
                        || candidate.logical_size() == 4 * 1024 * 1024;
                    page.done = next.is_none();
                    page.next = next;
                    page.total = total;
                    page.progress = progress;
                    page.first = false;
                    query.value = candidate;
                    query.status = "ready";
                    query.error = None;
                    query.stale = false;
                    query.ready = true;
                }
            }
        }
        true
    }
}
