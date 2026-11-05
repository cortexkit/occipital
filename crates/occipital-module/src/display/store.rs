//! The display store: what tool providers write for their calls
//! (`tool-display/v1` §2), in two SQLite files beside the module's store.
//!
//! Records, completions and views are backed up: a provider writes each
//! record only once, so one lost in a restore never comes back. Details are
//! not: they are large and can hold file contents. So details live in their
//! own file, which the Engram catalog leaves out. Each file has its own
//! storage namespace, so its single-writer lease never contends with another
//! file's.
//!
//! Every write is stored once per `(attested writer, runner, call key, kind)`
//! and never changes: the same canonical bytes again are `already_stored`,
//! different bytes are refused. Bodies are not checked against a view here,
//! since the view is fetched later and off the write path; the join checks
//! them.

use std::path::Path;
use std::sync::Arc;

use cortexkit_role_tool_display::{
    check_writer, idempotency_key, validate_write, write_outcome, DisplayKind, DisplayLimits,
    DisplayWrite, Principal, WriteReply,
};
use cortexkit_store::{open_sqlite, SqliteStore, StorageBackend, StorageDescriptor, StoreError};
use rusqlite::OptionalExtension;
use subc_protocol::ErrorBody;

use crate::store::OpenError;

/// The records file's name, in the module's data folder: records,
/// completions and views, backed up.
pub const FILE: &str = "display.db";

/// The details file's name, beside it, left out of backups.
pub const DETAILS_FILE: &str = "display-details.db";

/// The storage namespaces of the two files. The lease key is the module, the
/// backend and the namespace, never the path, so a second file under the main
/// store's namespace would wait on the lease this process already holds.
const NAMESPACE: &str = "display";
const DETAILS_NAMESPACE: &str = "display-details";

/// Writes, keyed by their idempotency key. Both files hold this table: the
/// records file the records and completions, the details file the details.
const WRITES: &str = "CREATE TABLE display_writes (
  key BLOB PRIMARY KEY NOT NULL, writer TEXT NOT NULL, runner TEXT NOT NULL,
  call_key TEXT NOT NULL, kind TEXT NOT NULL, schema_pin TEXT NOT NULL,
  body BLOB NOT NULL, stored_at_ms INTEGER NOT NULL) WITHOUT ROWID;
CREATE INDEX display_writes_call ON display_writes (runner, call_key);
CREATE INDEX display_writes_age ON display_writes (kind, stored_at_ms);";

/// The view each writer served for a pin, with the writer's `module_version`
/// it was fetched under (tool-provider §2.1). Views are never pruned: an old
/// call still needs its view to draw.
const VIEWS: &str = "CREATE TABLE display_views (
  writer TEXT NOT NULL, pin TEXT NOT NULL, state TEXT NOT NULL,
  manifest BLOB, input_schema BLOB, fetched_at_ms INTEGER NOT NULL,
  module_version TEXT NOT NULL, PRIMARY KEY (writer, pin)) WITHOUT ROWID;";

/// Each call's tool version as its runner stored it, keyed like the writes by
/// the runner and the call key, with the session the call belongs to. The
/// join must take a call's pin from the runner, never from a write or an app
/// (tool-display §6), and an app asks for records by call key alone, often
/// for a chat it cached before this process started. So the pin is kept here
/// when a page carrying the call is built from the runner's read.
const CALL_PINS: &str = "CREATE TABLE display_call_pins (
  runner TEXT NOT NULL, call_key TEXT NOT NULL, session TEXT NOT NULL,
  pin TEXT NOT NULL, seen_at_ms INTEGER NOT NULL,
  PRIMARY KEY (runner, call_key)) WITHOUT ROWID;
CREATE INDEX display_call_pins_age ON display_call_pins (seen_at_ms);";

/// One schema version for both files: the number of steps each opens with.
/// Version 2 adds the records file's call pins; the details file has nothing
/// new at 2.
#[cfg(test)]
const SCHEMA_VERSION: i64 = 2;

/// The limits this server serves in `role.describe.tool_display`: the
/// contract's ceilings for now. They come down only once a measured week of
/// real records and details says by how much (the design's §10).
pub fn served_limits() -> DisplayLimits {
    DisplayLimits::new(
        cortexkit_role_tool_display::MAX_RECORD_BYTES,
        cortexkit_role_tool_display::MAX_DETAIL_BYTES,
        cortexkit_role_tool_display::MAX_COMPLETION_BYTES,
        LOADING_WINDOW_MS,
    )
}

/// How long after a call's answer a reader shows its record as loading. A
/// provider writes asynchronously right after answering, so a record that
/// hasn't arrived within this is treated as not coming.
const LOADING_WINDOW_MS: u64 = 5_000;

/// How long writes are kept. Like the served limits, these are first figures,
/// to be set from a measured week of real writes (the design's §10).
#[derive(Debug, Clone, Copy)]
pub struct Retention {
    /// Records and completions: small, and a reopened chat needs them for as
    /// long as its history stays readable.
    pub record_age_ms: i64,
    /// Details: large, and only a tapped row reads one.
    pub detail_age_ms: i64,
    /// The most detail bytes kept; the oldest go first beyond it.
    pub detail_total_bytes: i64,
}

impl Retention {
    pub const DEFAULT: Self = Self {
        record_age_ms: 180 * DAY_MS,
        detail_age_ms: 14 * DAY_MS,
        detail_total_bytes: 512 * 1024 * 1024,
    };
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// What a writer answered for one pin, as kept for good.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewState {
    /// The writer's view, and the input schema of the catalog tool the pin
    /// names, both as canonical JSON bytes. Only this state authorizes a join.
    View {
        manifest: Vec<u8>,
        input_schema: Vec<u8>,
    },
    /// The writer answered `unknown`: it serves no view for the pin.
    Unknown,
    /// No tool in the writer's catalog has this pin, so its view, if any,
    /// can't be the writer's own (tool-display §6).
    NotInCatalog,
}

impl ViewState {
    fn label(&self) -> &'static str {
        match self {
            Self::View { .. } => "view",
            Self::Unknown => "unknown",
            Self::NotInCatalog => "not_in_catalog",
        }
    }
}

/// A kept answer with the writer's `module_version` it was fetched under.
#[derive(Debug, Clone, PartialEq)]
pub struct KeptView {
    pub state: ViewState,
    pub module_version: String,
    /// When the writer was last asked for this pin, in Unix milliseconds:
    /// the first fetch, a replacement or a check that kept the answer.
    pub asked_at_ms: i64,
}

/// One stored write, as the join reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    /// The route's attested opener when the write arrived.
    pub writer: Principal,
    pub kind: DisplayKind,
    /// The pin the writer says it served. The join never trusts it: it uses
    /// the pin the runner stored for the call.
    pub schema_pin: String,
    /// The body's canonical bytes, exactly as first written.
    pub body: Vec<u8>,
    pub stored_at_ms: i64,
}

pub struct DisplayStore {
    /// Records, completions and views: backed up.
    records: Arc<SqliteStore>,
    /// Details: left out of backups.
    details: Arc<SqliteStore>,
    limits: DisplayLimits,
}

impl DisplayStore {
    /// Open the two display files in the folder of the store `main` names,
    /// beside it.
    pub fn open(main: &StorageDescriptor, data_root: &Path) -> Result<Self, OpenError> {
        let records = open_file(
            main,
            data_root,
            NAMESPACE,
            FILE,
            &[&[WRITES, VIEWS], &[CALL_PINS]],
        )?;
        let details = open_file(
            main,
            data_root,
            DETAILS_NAMESPACE,
            DETAILS_FILE,
            &[&[WRITES], &[]],
        )?;
        Ok(Self {
            records: Arc::new(records),
            details: Arc::new(details),
            limits: served_limits(),
        })
    }

    /// The file a write of `kind` lives in.
    fn file_for(&self, kind: &DisplayKind) -> &SqliteStore {
        match kind {
            DisplayKind::Detail => &self.details,
            _ => &self.records,
        }
    }

    pub fn limits(&self) -> &DisplayLimits {
        &self.limits
    }

    /// Store `write` from `writer`, the route's attested opener, or say why not.
    /// The comparison and the insert run in one fenced transaction, so two
    /// writes racing for one key can't both be `stored`.
    pub fn write(
        &self,
        writer: &Principal,
        write: &DisplayWrite,
        now_ms: i64,
    ) -> Result<WriteReply, ErrorBody> {
        check_writer(writer)?;
        let body = validate_write(write, &self.limits)?;
        let key = idempotency_key(writer, write).map_err(|_| invalid("body"))?;
        let writer_text = principal_json(writer).map_err(|_| invalid("writer"))?;
        let runner_text = principal_json(&write.runner).map_err(|_| invalid("runner"))?;
        self.file_for(&write.kind)
            .with_conn_fenced(|tx| {
                let previous: Option<Vec<u8>> = tx
                    .query_row(
                        "SELECT body FROM display_writes WHERE key = ?1",
                        [&key],
                        |row| row.get(0),
                    )
                    .optional()?;
                let decided = write_outcome(previous.as_deref(), &body);
                if matches!(decided, Ok(WriteReply::Stored)) {
                    tx.execute(
                        "INSERT INTO display_writes
                         (key, writer, runner, call_key, kind, schema_pin, body, stored_at_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        rusqlite::params![
                            key,
                            writer_text,
                            runner_text,
                            write.call_key,
                            write.kind.as_str(),
                            write.schema_pin,
                            body,
                            now_ms
                        ],
                    )?;
                }
                Ok(decided)
            })
            .map_err(|error| unavailable(&error))?
    }

    /// Every stored write for one call of `runner`, from both files, oldest
    /// first.
    pub fn writes(&self, runner: &Principal, call_key: &str) -> Result<Vec<Stored>, StoreError> {
        let runner =
            principal_json(runner).map_err(|error| StoreError::Backend(error.to_string()))?;
        let mut keyed = Vec::new();
        for file in [&self.records, &self.details] {
            keyed.extend(file.with_conn(|conn| writes_in(conn, &runner, call_key))?);
        }
        keyed.sort_by(|(a_key, a), (b_key, b)| {
            (a.stored_at_ms, a_key).cmp(&(b.stored_at_ms, b_key))
        });
        Ok(keyed.into_iter().map(|(_, stored)| stored).collect())
    }

    /// Keep each call's pin as its runner stored it: `(runner, call key,
    /// session, pin)`. A call already kept stays, since a call's pin never
    /// changes.
    pub fn keep_call_pins(
        &self,
        calls: &[(String, String, String, String)],
        now_ms: i64,
    ) -> Result<(), StoreError> {
        if calls.is_empty() {
            return Ok(());
        }
        self.records.with_conn_fenced(|tx| {
            let mut insert = tx.prepare(
                "INSERT OR IGNORE INTO display_call_pins
                 (runner, call_key, session, pin, seen_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (runner, call_key, session, pin) in calls {
                insert.execute(rusqlite::params![runner, call_key, session, pin, now_ms])?;
            }
            Ok(())
        })
    }

    /// The session and pin kept for a call of `runner` (a module id).
    pub fn call_pin(
        &self,
        runner: &str,
        call_key: &str,
    ) -> Result<Option<(String, String)>, StoreError> {
        self.records.with_conn(|conn| {
            conn.query_row(
                "SELECT session, pin FROM display_call_pins WHERE runner = ?1 AND call_key = ?2",
                rusqlite::params![runner, call_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
        })
    }

    /// Every writer's kept answer for `pin`, by writer.
    pub fn views_for_pin(&self, pin: &str) -> Result<Vec<(String, KeptView)>, StoreError> {
        let writers: Vec<String> = self.records.with_conn(|conn| {
            let mut statement =
                conn.prepare("SELECT writer FROM display_views WHERE pin = ?1 ORDER BY writer")?;
            let writers = statement
                .query_map([pin], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            Ok(writers)
        })?;
        let mut kept = Vec::new();
        for writer in writers {
            if let Some(view) = self.kept_view(&writer, pin)? {
                kept.push((writer, view));
            }
        }
        Ok(kept)
    }

    /// Every kept view, by writer and pin, for finding a template by digest.
    pub fn all_views(&self) -> Result<Vec<(String, String, Vec<u8>)>, StoreError> {
        self.records.with_conn(|conn| {
            let mut statement = conn.prepare(
                "SELECT writer, pin, manifest FROM display_views
                 WHERE state = 'view' AND manifest IS NOT NULL ORDER BY writer, pin",
            )?;
            let views = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            Ok(views)
        })
    }

    /// What `writer` (a module id) answered for `pin`, or `None` when it
    /// hasn't been asked or the asking failed.
    pub fn view(&self, writer: &str, pin: &str) -> Result<Option<ViewState>, StoreError> {
        Ok(self.kept_view(writer, pin)?.map(|kept| kept.state))
    }

    /// The answer kept for `pin` with the writer's version it was fetched
    /// under, or `None` when it hasn't been asked or the asking failed.
    pub fn kept_view(&self, writer: &str, pin: &str) -> Result<Option<KeptView>, StoreError> {
        self.records.with_conn(|conn| {
            let kept = conn
                .query_row(
                    "SELECT state, manifest, input_schema, module_version, fetched_at_ms
                     FROM display_views WHERE writer = ?1 AND pin = ?2",
                    rusqlite::params![writer, pin],
                    |row| {
                        let state: String = row.get(0)?;
                        let state = match (state.as_str(), row.get(1)?, row.get(2)?) {
                            ("view", Some(manifest), Some(input_schema)) => ViewState::View {
                                manifest,
                                input_schema,
                            },
                            ("unknown", ..) => ViewState::Unknown,
                            ("not_in_catalog", ..) => ViewState::NotInCatalog,
                            // A row this build can't read is treated as never asked.
                            _ => return Ok(None),
                        };
                        Ok(Some(KeptView {
                            state,
                            module_version: row.get(3)?,
                            asked_at_ms: row.get(4)?,
                        }))
                    },
                )
                .optional()?;
            Ok(kept.flatten())
        })
    }

    /// Keep `writer`'s first answer for `pin`, fetched under `module_version`.
    /// An answer already kept stays: only [`DisplayStore::replace_view`]
    /// changes one, after comparing the two.
    pub fn keep_view(
        &self,
        writer: &str,
        pin: &str,
        state: &ViewState,
        module_version: &str,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        self.put_view(
            "INSERT OR IGNORE",
            writer,
            pin,
            state,
            module_version,
            now_ms,
        )
    }

    /// Replace the answer kept for `pin`: a view with a higher template
    /// revision, or a view where the writer had answered `unknown`.
    pub fn replace_view(
        &self,
        writer: &str,
        pin: &str,
        state: &ViewState,
        module_version: &str,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        self.put_view(
            "INSERT OR REPLACE",
            writer,
            pin,
            state,
            module_version,
            now_ms,
        )
    }

    /// Record that the answer kept for `pin` was checked again under
    /// `module_version` at `now_ms` and stays, so it isn't fetched again until
    /// the writer's version changes or it is due again by age.
    pub fn stamp_view(
        &self,
        writer: &str,
        pin: &str,
        module_version: &str,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        self.records.with_conn_fenced(|tx| {
            tx.execute(
                "UPDATE display_views SET module_version = ?3, fetched_at_ms = ?4
                 WHERE writer = ?1 AND pin = ?2",
                rusqlite::params![writer, pin, module_version, now_ms],
            )?;
            Ok(())
        })
    }

    fn put_view(
        &self,
        verb: &str,
        writer: &str,
        pin: &str,
        state: &ViewState,
        module_version: &str,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        let (manifest, input_schema) = match state {
            ViewState::View {
                manifest,
                input_schema,
            } => (Some(manifest), Some(input_schema)),
            _ => (None, None),
        };
        self.records.with_conn_fenced(|tx| {
            tx.execute(
                &format!(
                    "{verb} INTO display_views
                     (writer, pin, state, manifest, input_schema, fetched_at_ms, module_version)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
                ),
                rusqlite::params![
                    writer,
                    pin,
                    state.label(),
                    manifest,
                    input_schema,
                    now_ms,
                    module_version
                ],
            )?;
            Ok(())
        })
    }

    /// Drop records and completions past their age, and details past their
    /// age or beyond the total, oldest first. Returns how many writes went.
    pub fn prune(&self, now_ms: i64, retention: &Retention) -> Result<usize, StoreError> {
        let records = self.records.with_conn_fenced(|tx| {
            let cutoff = now_ms.saturating_sub(retention.record_age_ms);
            // A call's pin goes with its records: past that age the call has
            // nothing left to join.
            tx.execute(
                "DELETE FROM display_call_pins WHERE seen_at_ms < ?1",
                [cutoff],
            )?;
            tx.execute(
                "DELETE FROM display_writes WHERE stored_at_ms < ?1",
                [cutoff],
            )
        })?;
        let details = self.details.with_conn_fenced(|tx| {
            let mut removed = tx.execute(
                "DELETE FROM display_writes WHERE stored_at_ms < ?1",
                [now_ms.saturating_sub(retention.detail_age_ms)],
            )?;
            // The newest details whose running total stays within the limit stay.
            removed += tx.execute(
                "DELETE FROM display_writes WHERE key IN (
                   SELECT key FROM (
                     SELECT key, SUM(length(body)) OVER (
                       ORDER BY stored_at_ms DESC, key DESC) AS total
                     FROM display_writes)
                   WHERE total > ?1)",
                [retention.detail_total_bytes],
            )?;
            Ok(removed)
        })?;
        Ok(records + details)
    }
}

/// The writes for one call in one file, each with its key for ordering.
fn writes_in(
    conn: &rusqlite::Connection,
    runner: &str,
    call_key: &str,
) -> rusqlite::Result<Vec<(Vec<u8>, Stored)>> {
    let mut statement = conn.prepare(
        "SELECT key, writer, kind, schema_pin, body, stored_at_ms FROM display_writes
         WHERE runner = ?1 AND call_key = ?2",
    )?;
    let rows = statement.query_map(rusqlite::params![runner, call_key], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Vec<u8>>(4)?,
            row.get::<_, i64>(5)?,
        ))
    })?;
    let mut stored = Vec::new();
    for row in rows {
        let (key, writer, kind, schema_pin, body, stored_at_ms) = row?;
        // A writer that no longer decodes is skipped, never guessed.
        let Ok(writer) = serde_json::from_str(&writer) else {
            continue;
        };
        let Ok(kind) = serde_json::from_value(serde_json::Value::String(kind)) else {
            continue;
        };
        stored.push((
            key,
            Stored {
                writer,
                kind,
                schema_pin,
                body,
                stored_at_ms,
            },
        ));
    }
    Ok(stored)
}

/// Open a file beside the main store, under its own storage namespace so its
/// lease never contends with another file's, and bring its schema up to date.
/// `steps[n]` holds the tables version `n + 1` adds, so the file's version is
/// the number of steps. A file at a version this build doesn't know is
/// refused. The display files and the notices file (`crate::notices`) open
/// this way.
pub(crate) fn open_file(
    main: &StorageDescriptor,
    data_root: &Path,
    namespace: &str,
    file: &str,
    steps: &[&[&str]],
) -> Result<SqliteStore, OpenError> {
    let version = i64::try_from(steps.len()).unwrap_or(i64::MAX);
    let descriptor = StorageDescriptor {
        module_id: main.module_id.clone(),
        storage_namespace: namespace.into(),
        isolation: main.isolation.clone(),
        backend: StorageBackend::Sqlite {
            path: data_root.join(file).to_string_lossy().into_owned(),
        },
    };
    let sqlite = open_sqlite(&descriptor)?;
    let found = sqlite.with_conn(user_version)?;
    if !(0..=version).contains(&found) {
        return Err(OpenError::VersionUnsupported { found });
    }
    sqlite.with_conn_fenced(|tx| {
        let found = user_version(tx)?;
        if found < version {
            for step in steps.iter().skip(usize::try_from(found).unwrap_or(0)) {
                for table in *step {
                    tx.execute_batch(table)?;
                }
            }
            tx.pragma_update(None, "user_version", version)?;
        }
        Ok(())
    })?;
    Ok(sqlite)
}

/// How often the pruning task runs. The first run is at startup.
const PRUNE_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// Prune now and then every [`PRUNE_EVERY`], for the module's whole life. A
/// failed run is logged and the next one tries again, so the task never ends:
/// health reports a finished background task as stopped work.
pub fn start_pruning(store: Arc<DisplayStore>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(PRUNE_EVERY);
        loop {
            every.tick().await;
            let store = store.clone();
            let run =
                tokio::task::spawn_blocking(move || store.prune(now_ms(), &Retention::DEFAULT));
            match run.await {
                Ok(Ok(0)) => {}
                Ok(Ok(removed)) => tracing::info!(removed, "pruned display writes"),
                Ok(Err(error)) => tracing::warn!(%error, "cannot prune display writes"),
                Err(error) => tracing::warn!(%error, "display pruning stopped early"),
            }
        }
    })
}

/// The time a write is stored at, in Unix milliseconds. A clock before 1970
/// reads as 0, which only makes such writes the first to be pruned.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_millis()).ok())
        .unwrap_or(0)
}

fn user_version(conn: &rusqlite::Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

/// A principal in its canonical JSON form, so equal principals compare equal.
fn principal_json(principal: &Principal) -> Result<String, serde_json::Error> {
    serde_jcs::to_string(principal)
}

fn invalid(field: &str) -> ErrorBody {
    cortexkit_role_tool_display::InvalidParams::new(field).refusal()
}

/// A store failure, as the provider sees it: retryable, and the write wasn't
/// stored. Providers may retry it or drop it, as for any refused write.
fn unavailable(error: &dyn std::fmt::Display) -> ErrorBody {
    ErrorBody::new(
        "provider_unavailable",
        format!("display store unavailable: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortexkit_test_support::ScratchDir;
    use serde_json::{json, Map, Value};

    fn reserved(module: &str) -> Principal {
        Principal::Reserved {
            module_id: module.into(),
        }
    }

    fn body(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn record(call: &str, value: Value) -> DisplayWrite {
        DisplayWrite::new(
            reserved("broca"),
            call,
            DisplayKind::Record,
            "tp1:bash:0123:1",
            body(value),
        )
    }

    /// The display file opened beside a main store in the same process, as at
    /// startup: each holds its own lease.
    fn opened(dir: &ScratchDir) -> (crate::store::Store, DisplayStore) {
        let descriptor = crate::store::tests::descriptor(dir);
        let main = crate::store::open(&descriptor).unwrap();
        let display = DisplayStore::open(&descriptor, main.data_root()).unwrap();
        (main, display)
    }

    #[test]
    fn a_write_is_stored_once_and_never_replaced() {
        let dir = ScratchDir::new("occipital-display-once");
        let (_main, store) = opened(&dir);
        let first = record("call-1", json!({"durationMs": 5, "outcome": "success"}));
        assert_eq!(
            store.write(&reserved("aft"), &first, 100),
            Ok(WriteReply::Stored)
        );
        // The same content in another member order is the same canonical bytes.
        let reordered = record("call-1", json!({"outcome": "success", "durationMs": 5}));
        assert_eq!(
            store.write(&reserved("aft"), &reordered, 200),
            Ok(WriteReply::AlreadyStored)
        );
        let changed = record("call-1", json!({"durationMs": 6, "outcome": "success"}));
        let refused = store.write(&reserved("aft"), &changed, 300).unwrap_err();
        assert_eq!(refused.code, "display_write_conflict");
        let stored = store.writes(&reserved("broca"), "call-1").unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].body, br#"{"durationMs":5,"outcome":"success"}"#);
        assert_eq!(stored[0].stored_at_ms, 100);
        assert_eq!(stored[0].writer, reserved("aft"));
        assert_eq!(stored[0].schema_pin, "tp1:bash:0123:1");
    }

    /// Another module's write for the same call takes its own slot, so it can't
    /// block the real provider's write; the join picks the writer later.
    #[test]
    fn another_writer_gets_its_own_slot_for_the_same_call() {
        let dir = ScratchDir::new("occipital-display-writers");
        let (_main, store) = opened(&dir);
        let squatter = record("call-1", json!({"durationMs": 1, "outcome": "error"}));
        let real = record("call-1", json!({"durationMs": 5, "outcome": "success"}));
        assert_eq!(
            store.write(&reserved("intruder"), &squatter, 1),
            Ok(WriteReply::Stored)
        );
        assert_eq!(
            store.write(&reserved("aft"), &real, 2),
            Ok(WriteReply::Stored)
        );
        let writers: Vec<_> = store
            .writes(&reserved("broca"), "call-1")
            .unwrap()
            .into_iter()
            .map(|stored| stored.writer)
            .collect();
        assert_eq!(writers, [reserved("intruder"), reserved("aft")]);
        // Another runner's call with the same key is a different call.
        assert!(store
            .writes(&reserved("thalamus-harness"), "call-1")
            .unwrap()
            .is_empty());
    }

    /// Keeping never overwrites the first answer for a writer's pin, and each
    /// writer's answer is its own; only a replace changes one, and a stamp
    /// changes only the version it was checked under and when it was asked.
    #[test]
    fn the_first_view_kept_stays_until_replaced() {
        let dir = ScratchDir::new("occipital-display-views");
        let (_main, store) = opened(&dir);
        let view = ViewState::View {
            manifest: b"{}".to_vec(),
            input_schema: b"{}".to_vec(),
        };
        assert_eq!(store.view("aft", "pin").unwrap(), None);
        store
            .keep_view("aft", "pin", &ViewState::Unknown, "1.0", 1)
            .unwrap();
        store.keep_view("aft", "pin", &view, "1.0", 2).unwrap();
        store.keep_view("core", "pin", &view, "2.0", 3).unwrap();
        assert_eq!(store.view("aft", "pin").unwrap(), Some(ViewState::Unknown));
        assert_eq!(store.view("core", "pin").unwrap(), Some(view.clone()));

        let kept = |writer| store.kept_view(writer, "pin").unwrap().unwrap();
        assert_eq!(kept("aft").asked_at_ms, 1);
        store.stamp_view("aft", "pin", "1.1", 5).unwrap();
        assert_eq!(
            kept("aft"),
            KeptView {
                state: ViewState::Unknown,
                module_version: "1.1".into(),
                asked_at_ms: 5,
            }
        );
        store.replace_view("aft", "pin", &view, "1.2", 6).unwrap();
        assert_eq!(
            kept("aft"),
            KeptView {
                state: view,
                module_version: "1.2".into(),
                asked_at_ms: 6,
            }
        );
        assert_eq!(kept("core").module_version, "2.0");
    }

    /// Records and completions go to the backed-up file and details to the
    /// other, and a call's writes read back from both, oldest first.
    #[test]
    fn details_live_apart_from_records() {
        let dir = ScratchDir::new("occipital-display-split");
        let (main, store) = opened(&dir);
        let finished = json!({"outcome": "success", "durationMs": 5});
        for (kind, value, at) in [
            (DisplayKind::Detail, json!({"text": "line"}), 1),
            (DisplayKind::Record, finished.clone(), 2),
            (DisplayKind::Completion, finished, 3),
        ] {
            let write = DisplayWrite::new(
                reserved("broca"),
                "call-1",
                kind,
                "tp1:bash:0123:1",
                body(value),
            );
            assert_eq!(
                store.write(&reserved("aft"), &write, at),
                Ok(WriteReply::Stored)
            );
        }
        let kinds_in = |file: &str| {
            let conn = rusqlite::Connection::open(main.data_root().join(file)).unwrap();
            let mut statement = conn
                .prepare("SELECT kind FROM display_writes ORDER BY kind")
                .unwrap();
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>()
        };
        assert_eq!(kinds_in(FILE), ["completion", "record"]);
        assert_eq!(kinds_in(DETAILS_FILE), ["detail"]);
        let kinds: Vec<DisplayKind> = store
            .writes(&reserved("broca"), "call-1")
            .unwrap()
            .into_iter()
            .map(|stored| stored.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                DisplayKind::Detail,
                DisplayKind::Record,
                DisplayKind::Completion
            ]
        );
    }

    #[test]
    fn refused_writes_store_nothing() {
        let dir = ScratchDir::new("occipital-display-refused");
        let (_main, store) = opened(&dir);
        let write = record("call-1", json!({"durationMs": 5, "outcome": "success"}));
        for writer in [Principal::Direct, Principal::Unverified] {
            let refused = store.write(&writer, &write, 1).unwrap_err();
            assert_eq!(refused.code, "display_writer_unverified", "{writer:?}");
        }
        let large = record(
            "call-1",
            json!({"durationMs": 5, "outcome": "success", "x": "a".repeat(20_000)}),
        );
        let refused = store.write(&reserved("aft"), &large, 1).unwrap_err();
        assert_eq!(refused.code, "display_too_large");
        let mut unknown = write.clone();
        unknown.kind = DisplayKind::Unknown("summary".into());
        let refused = store.write(&reserved("aft"), &unknown, 1).unwrap_err();
        assert_eq!(refused.code, "invalid_params");
        assert_eq!(refused.detail, Some(json!({"field": "kind"})));
        assert!(store
            .writes(&reserved("broca"), "call-1")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn pruning_drops_old_writes_and_the_oldest_details_beyond_the_total() {
        let dir = ScratchDir::new("occipital-display-prune");
        let (_main, store) = opened(&dir);
        let detail = |call: &str, size: usize| {
            DisplayWrite::new(
                reserved("broca"),
                call,
                DisplayKind::Detail,
                "tp1:edit:0123:1",
                body(json!({"diff": "d".repeat(size)})),
            )
        };
        let aft = reserved("aft");
        store
            .write(
                &aft,
                &record("old", json!({"durationMs": 1, "outcome": "success"})),
                0,
            )
            .unwrap();
        store
            .write(
                &aft,
                &record("new", json!({"durationMs": 1, "outcome": "success"})),
                900,
            )
            .unwrap();
        for (call, at) in [("d1", 600), ("d2", 700), ("d3", 800)] {
            store.write(&aft, &detail(call, 100), at).unwrap();
        }
        let retention = Retention {
            record_age_ms: 500,
            detail_age_ms: 350,
            detail_total_bytes: 250,
        };
        // At 1,000: "old" is past the record age, d1 past the detail age, and of
        // d2 and d3 (about 111 bytes each) only the newest two fit 250 bytes.
        assert_eq!(store.prune(1_000, &retention).unwrap(), 2);
        let kept = |call: &str| !store.writes(&reserved("broca"), call).unwrap().is_empty();
        assert!(!kept("old") && kept("new"));
        assert!(!kept("d1") && kept("d2") && kept("d3"));
        let tight = Retention {
            detail_total_bytes: 150,
            ..retention
        };
        assert_eq!(store.prune(1_000, &tight).unwrap(), 1);
        assert!(!kept("d2") && kept("d3"));
    }

    /// A call's pin is kept once, read back by runner and call key, and pruned
    /// with the records.
    #[test]
    fn a_calls_pin_is_kept_once_and_pruned_with_its_records() {
        let dir = ScratchDir::new("occipital-display-call-pins");
        let (_main, store) = opened(&dir);
        let call = |runner: &str, key: &str, pin: &str| {
            (
                runner.to_owned(),
                key.to_owned(),
                "ses_1".to_owned(),
                pin.to_owned(),
            )
        };
        store
            .keep_call_pins(&[call("thalamus-harness", "ck_1", "tp1:bash:00:1")], 10)
            .unwrap();
        // A second sighting with another pin changes nothing: a call's pin
        // never changes, so the first one stays.
        store
            .keep_call_pins(
                &[
                    call("thalamus-harness", "ck_1", "tp1:bash:00:2"),
                    call("broca", "ck_1", "tp1:read:00:1"),
                ],
                900,
            )
            .unwrap();
        assert_eq!(
            store.call_pin("thalamus-harness", "ck_1").unwrap(),
            Some(("ses_1".into(), "tp1:bash:00:1".into()))
        );
        assert_eq!(
            store.call_pin("broca", "ck_1").unwrap(),
            Some(("ses_1".into(), "tp1:read:00:1".into()))
        );
        assert_eq!(store.call_pin("broca", "ck_2").unwrap(), None);
        store
            .prune(
                1_000,
                &Retention {
                    record_age_ms: 500,
                    ..Retention::DEFAULT
                },
            )
            .unwrap();
        assert_eq!(store.call_pin("thalamus-harness", "ck_1").unwrap(), None);
        assert!(store.call_pin("broca", "ck_1").unwrap().is_some());
    }

    #[test]
    fn views_are_listed_by_pin_and_in_whole() {
        let dir = ScratchDir::new("occipital-display-view-lists");
        let (_main, store) = opened(&dir);
        let view = |manifest: &[u8]| ViewState::View {
            manifest: manifest.to_vec(),
            input_schema: b"{}".to_vec(),
        };
        store
            .keep_view("core", "pin-a", &view(b"{\"c\":1}"), "1", 1)
            .unwrap();
        store
            .keep_view("aft", "pin-a", &ViewState::NotInCatalog, "1", 1)
            .unwrap();
        store
            .keep_view("aft", "pin-b", &view(b"{\"a\":1}"), "1", 1)
            .unwrap();
        store
            .keep_view("aft", "pin-c", &ViewState::Unknown, "1", 1)
            .unwrap();
        let writers: Vec<_> = store
            .views_for_pin("pin-a")
            .unwrap()
            .into_iter()
            .map(|(writer, kept)| (writer, kept.state))
            .collect();
        assert_eq!(
            writers,
            [
                ("aft".to_owned(), ViewState::NotInCatalog),
                ("core".to_owned(), view(b"{\"c\":1}"))
            ]
        );
        assert_eq!(
            store.all_views().unwrap(),
            [
                ("aft".to_owned(), "pin-b".to_owned(), b"{\"a\":1}".to_vec()),
                ("core".to_owned(), "pin-a".to_owned(), b"{\"c\":1}".to_vec())
            ]
        );
    }

    /// A version 1 records file, which has no call pins table, gains it and
    /// keeps every write and view.
    #[test]
    fn a_version_1_file_is_brought_up_to_date_and_keeps_its_rows() {
        let dir = ScratchDir::new("occipital-display-upgrade");
        let descriptor = crate::store::tests::descriptor(&dir);
        let main = crate::store::open(&descriptor).unwrap();
        {
            let conn = rusqlite::Connection::open(main.data_root().join(FILE)).unwrap();
            conn.execute_batch(WRITES).unwrap();
            conn.execute_batch(VIEWS).unwrap();
            conn.execute(
                "INSERT INTO display_views VALUES ('aft', 'pin', 'unknown', NULL, NULL, 1, 'v1')",
                [],
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
        }
        let store = DisplayStore::open(&descriptor, main.data_root()).unwrap();
        assert_eq!(store.view("aft", "pin").unwrap(), Some(ViewState::Unknown));
        store
            .keep_call_pins(
                &[("broca".into(), "ck".into(), "s".into(), "pin".into())],
                1,
            )
            .unwrap();
        assert!(store.call_pin("broca", "ck").unwrap().is_some());
        for file in [FILE, DETAILS_FILE] {
            let conn = rusqlite::Connection::open(main.data_root().join(file)).unwrap();
            assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION, "{file}");
        }
    }
}
