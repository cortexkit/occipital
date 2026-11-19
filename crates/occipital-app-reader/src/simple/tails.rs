//! Finding a chat's Simple rows, and keeping them between reads.
//!
//! The view shows a few messages in hundreds, so a Simple page is found by
//! reading the full history back from its cursor and keeping what
//! [`super::admit`] shows. One read stops at its `limit` of rows, at the
//! start, or after a bound of messages read, and says where it stopped, so
//! the app goes on from there.
//!
//! A watch checks its claim on every change of a busy chat, and reading the
//! window again each time would cost a few hundred messages per step. So each
//! chat's Simple rows are kept with the range of history they were found in,
//! per history version ([`Tail`]). A newest page reads only what was added
//! since, plus the newest [`RECHECK`] messages again, since a step's row can
//! still change while its calls run; an older page beyond the kept range
//! reads on and extends it. The kept rows go when the version moves, when a
//! `say` record that one of them draws is written ([`Tails::forget_call`]),
//! and at restart.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::source::{Anchor, Head, HistoryPage, ReadError, Row, TranscriptSource};

/// Messages read from the source at a time.
pub const SCAN_PAGE: u32 = 100;
/// The most messages one older page reads.
pub const BACKWARD_BOUND: u64 = 1_000;
/// The most messages a newest page or a claim reads on a chat with nothing
/// kept yet.
pub const NEWEST_BOUND: u64 = 2_000;
/// The newest messages a newest page reads again, since a step's row changes
/// in place while its calls run and their results land as later entries.
pub const RECHECK: u64 = 50;
/// The most Simple rows kept for one chat; past it the oldest go.
const MAX_ROWS: usize = 2_000;
/// The most chats kept; past it the one read longest ago goes.
const MAX_CHATS: usize = 64;

/// The Simple rows a read found, oldest first, and where an older page goes
/// on: before `next_before`, or nowhere when the read reached the start.
#[derive(Debug, Clone)]
pub struct Found {
    pub rows: Vec<Row>,
    pub next_before: Option<u64>,
}

/// A chat's Simple rows at one version, and the range of messages, `from`
/// through `to`, they were found in: every Simple row in that range is here.
#[derive(Debug, Clone)]
struct Tail {
    version: String,
    rows: Vec<Row>,
    from: u64,
    to: u64,
    at_start: bool,
    used: u64,
}

/// Each chat's kept Simple rows.
#[derive(Debug, Default)]
pub struct Tails {
    tails: Mutex<HashMap<String, Tail>>,
    uses: AtomicU64,
    invalidations: AtomicU64,
}

/// The ordinal of a numbered row.
fn ordinal(row: &Row) -> u64 {
    row.ordinal.unwrap_or_default()
}

impl Tails {
    /// The newest `limit` Simple rows of `session` as `head` names it.
    pub async fn newest(
        &self,
        source: &dyn TranscriptSource,
        session: &str,
        head: &Head,
        version: &str,
        limit: usize,
    ) -> Result<Found, ReadError> {
        let Some(newest) = head.last_ordinal else {
            return Ok(Found {
                rows: Vec::new(),
                next_before: None,
            });
        };
        let generation = self.invalidations.load(Ordering::Acquire);
        let mut read = 0;
        let refreshed = match self.kept(session, version).filter(|tail| {
            tail.to <= newest
                && newest.saturating_sub(
                    tail.to
                        .saturating_sub(RECHECK)
                        .max(tail.from)
                        .saturating_sub(1),
                ) <= NEWEST_BOUND
        }) {
            Some(mut tail) => {
                let from = tail.to.saturating_sub(RECHECK).max(tail.from);
                match after(source, head, from.saturating_sub(1), &mut read).await {
                    Ok(fresh) => {
                        tail.rows.retain(|row| ordinal(row) < from);
                        tail.rows.extend(fresh.rows);
                        tail.to = fresh.to.max(tail.to);
                        Some(tail)
                    }
                    // A source that can't read after an ordinal reads its
                    // newest rows afresh each time.
                    Err(error) if error.code == "unsupported" => None,
                    Err(error) => return Err(error),
                }
            }
            None => None,
        };
        let mut tail = match refreshed {
            Some(tail) => tail,
            None => {
                let mut tail = Tail {
                    version: version.to_owned(),
                    rows: Vec::new(),
                    from: newest.saturating_add(1),
                    to: newest,
                    at_start: false,
                    used: 0,
                };
                back(source, head, &mut tail, limit, NEWEST_BOUND, &mut read).await?;
                tail
            }
        };
        if tail.rows.len() < limit && !tail.at_start {
            back(source, head, &mut tail, limit, NEWEST_BOUND, &mut read).await?;
        }
        let found = serve(&tail, u64::MAX, limit);
        self.keep(session, tail, generation);
        Ok(found)
    }

    /// The `limit` Simple rows of `session` just before ordinal `before`.
    pub async fn older(
        &self,
        source: &dyn TranscriptSource,
        session: &str,
        head: &Head,
        version: &str,
        before: u64,
        limit: usize,
    ) -> Result<Found, ReadError> {
        let generation = self.invalidations.load(Ordering::Acquire);
        let mut read = 0;
        match self
            .kept(session, version)
            .filter(|tail| tail.from <= before && before <= tail.to.saturating_add(1))
        {
            Some(mut tail) => {
                let held = tail.rows.iter().filter(|row| ordinal(row) < before).count();
                if held < limit && !tail.at_start {
                    let wanted = tail.rows.len() - held + limit;
                    back(source, head, &mut tail, wanted, BACKWARD_BOUND, &mut read).await?;
                }
                let found = serve(&tail, before, limit);
                self.keep(session, tail, generation);
                Ok(found)
            }
            // Outside the kept range, the page is read on its own and kept
            // nowhere: it joins no range this chat keeps.
            None => {
                let mut range = Tail {
                    version: version.to_owned(),
                    rows: Vec::new(),
                    from: before,
                    to: before.saturating_sub(1),
                    at_start: before <= 1,
                    used: 0,
                };
                if !range.at_start {
                    back(source, head, &mut range, limit, BACKWARD_BOUND, &mut read).await?;
                }
                Ok(serve(&range, before, limit))
            }
        }
    }

    /// Forget every chat whose kept rows draw the call `call_key`: its record
    /// was written after the row was kept, so the row is read again.
    pub fn forget_call(&self, call_key: &str) {
        let mut tails = self
            .tails
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.invalidations.fetch_add(1, Ordering::Release);
        tails.retain(|_, tail| !tail.rows.iter().any(|row| draws_call(row, call_key)));
    }

    /// Forget every chat's kept rows, as after missed news of records.
    pub fn forget_all(&self) {
        let mut tails = self
            .tails
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.invalidations.fetch_add(1, Ordering::Release);
        tails.clear();
    }

    /// Forget `session`'s kept rows.
    pub fn forget(&self, session: &str) {
        let mut tails = self
            .tails
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.invalidations.fetch_add(1, Ordering::Release);
        tails.remove(session);
    }

    fn kept(&self, session: &str, version: &str) -> Option<Tail> {
        self.tails
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(session)
            .filter(|tail| tail.version == version)
            .cloned()
    }

    /// Keep `tail` for `session`, unless a read that finished meanwhile kept
    /// a newer one, at most [`MAX_ROWS`] rows of it, and at most
    /// [`MAX_CHATS`] chats.
    fn keep(&self, session: &str, mut tail: Tail, generation: u64) {
        if tail.rows.len() > MAX_ROWS {
            let dropped = tail.rows.len() - MAX_ROWS;
            tail.from = ordinal(&tail.rows[dropped - 1]).saturating_add(1);
            tail.rows.drain(..dropped);
            tail.at_start = false;
        }
        tail.used = self.uses.fetch_add(1, Ordering::Relaxed);
        let mut tails = self
            .tails
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        // A record may have arrived while this read was outside the lock, even
        // on a cold read whose rows were not yet in the map to be evicted.
        if self.invalidations.load(Ordering::Acquire) != generation {
            return;
        }
        if let Some(kept) = tails.get(session) {
            if kept.version == tail.version && kept.to > tail.to {
                return;
            }
        }
        if !tails.contains_key(session) && tails.len() >= MAX_CHATS {
            let oldest = tails
                .iter()
                .min_by_key(|(_, tail)| tail.used)
                .map(|(session, _)| session.clone());
            if let Some(oldest) = oldest {
                tails.remove(&oldest);
            }
        }
        tails.insert(session.to_owned(), tail);
    }
}

/// Whether `row` holds a part for the call `call_key`.
fn draws_call(row: &Row, call_key: &str) -> bool {
    row.display["parts"]
        .as_array()
        .is_some_and(|parts| parts.iter().any(|part| part["callKey"] == call_key))
}

/// The `limit` newest of `tail`'s rows before `before`, and where an older
/// page goes on: before the oldest row served while the tail holds older
/// ones, else where the tail's range starts, else nowhere at the start.
fn serve(tail: &Tail, before: u64, limit: usize) -> Found {
    let below: Vec<&Row> = tail
        .rows
        .iter()
        .filter(|row| ordinal(row) < before)
        .collect();
    let start = below.len().saturating_sub(limit);
    let rows: Vec<Row> = below[start..].iter().map(|row| (*row).clone()).collect();
    let next_before = if start > 0 {
        rows.first().map(ordinal)
    } else if tail.at_start {
        None
    } else {
        Some(tail.from)
    };
    Found { rows, next_before }
}

/// The Simple rows among `page`'s, each trimmed to what the view draws.
async fn admitted(source: &dyn TranscriptSource, page: Vec<Row>) -> Vec<Row> {
    let asks: HashSet<String> = source.answered_asks(&page).await;
    page.into_iter()
        .filter_map(|mut row| {
            let answers = asks.contains(&row.mid);
            super::admit(&mut row.display, answers).then_some(row)
        })
        .collect()
}

/// Refuse a page from a history other than the one `head` names: rows of two
/// versions can't be one copy.
fn agreed(page: &HistoryPage, head: &Head) -> Result<(), ReadError> {
    if page.history_version == head.history_version && page.incarnation == head.incarnation {
        Ok(())
    } else {
        Err(ReadError::new(
            "provider_unavailable",
            "the session's history changed while its Simple rows were read",
        ))
    }
}

/// Read `tail` on into older history from its `from`, until it holds
/// `wanted` rows, reaches the start, or this read has read `bound` messages
/// in all (`read` counts them across one app request).
async fn back(
    source: &dyn TranscriptSource,
    head: &Head,
    tail: &mut Tail,
    wanted: usize,
    bound: u64,
    read: &mut u64,
) -> Result<(), ReadError> {
    let first = head.first_ordinal.unwrap_or(1);
    while tail.rows.len() < wanted && !tail.at_start && *read < bound {
        // A read from past the newest message is a read of the newest: a
        // runner refuses an ordinal anchor it doesn't hold.
        let page = if head.last_ordinal.is_some_and(|last| tail.from > last) {
            source.newest(SCAN_PAGE).await?
        } else {
            source.older(&Anchor::Ordinal(tail.from), SCAN_PAGE).await?
        };
        agreed(&page, head)?;
        // Unnumbered history is outside the view's reach: a Simple page goes
        // back by ordinal only.
        let numbered: Vec<Row> = page
            .rows
            .into_iter()
            .filter(|row| row.ordinal.is_some_and(|at| at < tail.from))
            .collect();
        *read = read.saturating_add(numbered.len() as u64);
        let oldest = numbered.first().map(ordinal).or_else(|| {
            let before = page.empty_before.filter(|at| *at < tail.from)?;
            *read = read.saturating_add(tail.from.saturating_sub(before));
            Some(before)
        });
        let mut found = admitted(source, numbered).await;
        found.append(&mut tail.rows);
        tail.rows = found;
        match oldest {
            Some(oldest) => {
                tail.from = oldest;
                tail.at_start = oldest <= first;
            }
            None => tail.at_start = true,
        }
    }
    Ok(())
}

/// Everything after ordinal `ordinal` through the newest message: its Simple
/// rows and the newest ordinal read.
struct After {
    rows: Vec<Row>,
    to: u64,
}

async fn after(
    source: &dyn TranscriptSource,
    head: &Head,
    ordinal: u64,
    read: &mut u64,
) -> Result<After, ReadError> {
    let mut next = ordinal;
    let mut rows = Vec::new();
    loop {
        let through = head.last_ordinal.unwrap_or(next);
        let limit = through.saturating_sub(next).min(u64::from(SCAN_PAGE)) as u32;
        if limit == 0 {
            return Ok(After { rows, to: next });
        }
        let positioned = source.after(next, limit).await?;
        agreed(&positioned.page, head)?;
        let page: Vec<Row> = positioned
            .page
            .rows
            .into_iter()
            .filter(|row| row.ordinal.is_some_and(|at| at > next && at <= through))
            .collect();
        *read = read.saturating_add(page.len() as u64);
        let last = page.last().map(self::ordinal);
        rows.extend(admitted(source, page).await);
        match (last, positioned.newest) {
            (Some(last), Some(newest)) if last < newest.min(through) => next = last,
            (Some(last), _) => {
                return Ok(After { rows, to: last });
            }
            (None, _) => return Ok(After { rows, to: next }),
        }
    }
}

#[cfg(test)]
mod tests;
