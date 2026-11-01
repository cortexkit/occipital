//! Occipital's store: one SQLite file the daemon names in HELLO_ACK, opened
//! with cortexkit-store so the single-writer lease, the write fence and the
//! owner-only file protection are the shared crate's, not this module's.
//!
//! The `show_ui` records' schema version lives in SQLite's `user_version`.
//! The app reader's tables (send receipts, unsent sends, uploads) keep their
//! own cortexkit-store migration chains in the same file, each refusing a
//! store ahead of it. Migrations only move forward. A store written by a newer
//! build stops startup instead of being served, because this build cannot know
//! what the newer columns mean.

use cortexkit_store::{open_sqlite, SqliteStore, StorageBackend, StorageDescriptor, StoreError};
use subc_protocol::ModuleHelloAckBody;

use crate::startup::{Fatal, EXIT_STORE_FAILURE};

/// The start of the stderr line written when the store is newer than this build.
pub const VERSION_UNSUPPORTED_PREFIX: &str = "ck-occipital: store version unsupported:";

/// The start of the stderr line written when the store cannot be opened.
pub const UNAVAILABLE_PREFIX: &str = "ck-occipital: store unavailable:";

/// Version 1: the records of native `show_ui` calls.
const NATIVE_RECORDS_V1: &str = "CREATE TABLE native_records (
  record_id TEXT PRIMARY KEY NOT NULL, owner TEXT NOT NULL, ref TEXT NOT NULL,
  scope_epoch INTEGER NOT NULL, principal TEXT NOT NULL, created_at_ms INTEGER NOT NULL,
  title TEXT NOT NULL, summary TEXT NOT NULL, source BLOB NOT NULL,
  source_hash TEXT NOT NULL, data TEXT, data_format TEXT, data_columns TEXT,
  output_refs BLOB, execution_dialect TEXT NOT NULL, catalog_version TEXT NOT NULL,
  catalog_digest TEXT NOT NULL, catalog_layout TEXT NOT NULL, fallback TEXT NOT NULL,
  fallback_truncated INTEGER NOT NULL, now_ms INTEGER NOT NULL, zone TEXT NOT NULL);";

/// Every migration, in order: entry `i` moves the store from `user_version`
/// `i` to `i + 1`. A later schema appends an entry and never edits one.
const MIGRATIONS: &[&str] = &[NATIVE_RECORDS_V1];

/// The newest schema version this build knows.
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// Why the store could not be served.
#[derive(Debug)]
pub enum OpenError {
    /// HELLO_ACK named no storage, so there is nowhere to keep records.
    NoDescriptor,
    /// HELLO_ACK's storage member is not a storage descriptor.
    BadDescriptor(String),
    /// The file's `user_version` is one this build does not know.
    VersionUnsupported { found: i64 },
    /// cortexkit-store refused to open or write the store.
    Store(StoreError),
}

impl OpenError {
    /// The line this failure writes and the status it exits with.
    pub fn fatal(&self) -> Fatal {
        match self {
            Self::NoDescriptor => Fatal::new(
                EXIT_STORE_FAILURE,
                UNAVAILABLE_PREFIX,
                "HELLO_ACK carries no storage descriptor; configure managed storage for occipital",
            ),
            Self::BadDescriptor(error) => Fatal::new(
                EXIT_STORE_FAILURE,
                UNAVAILABLE_PREFIX,
                format!("HELLO_ACK's storage descriptor is invalid: {error}"),
            ),
            Self::VersionUnsupported { found } => Fatal::new(
                EXIT_STORE_FAILURE,
                VERSION_UNSUPPORTED_PREFIX,
                format!(
                    "the store has user_version {found}, and this build knows versions up to \
                     {SCHEMA_VERSION}; run a newer occipital"
                ),
            ),
            Self::Store(error) => Fatal::new(EXIT_STORE_FAILURE, UNAVAILABLE_PREFIX, error),
        }
    }
}

impl From<StoreError> for OpenError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// The storage descriptor HELLO_ACK carries. There is no fallback path: the
/// daemon resolves `<data_home>/cortexkit/occipital/store.db` itself, and a
/// module guessing its own data home could split from the directory its
/// supervisor serves.
pub fn descriptor_from_ack(ack: &ModuleHelloAckBody) -> Result<StorageDescriptor, OpenError> {
    let value = ack.storage.clone().ok_or(OpenError::NoDescriptor)?;
    serde_json::from_value(value).map_err(|error| OpenError::BadDescriptor(error.to_string()))
}

/// An open, migrated store.
pub struct Store {
    sqlite: std::sync::Arc<SqliteStore>,
    data_root: std::path::PathBuf,
    file_name: String,
}

impl Store {
    /// The open store, for the app reader's tables beside the records.
    pub fn shared(&self) -> std::sync::Arc<SqliteStore> {
        self.sqlite.clone()
    }

    /// The folder holding the store file: the module's own data folder, where
    /// uploads keep their partial files.
    pub fn data_root(&self) -> &std::path::Path {
        &self.data_root
    }

    /// The store file's name in [`Self::data_root`], as the daemon named it.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// The fence epoch of the lease this process holds: every fenced write
    /// carries it, so a superseded process cannot write after its successor.
    pub fn epoch(&self) -> u64 {
        self.sqlite.epoch()
    }

    /// Commit only after the executor has serialized and bounded the reply.
    /// The lease fence and row insertion share a transaction, so a failed fence
    /// cannot leave a record that the caller was told had failed.
    #[cfg(feature = "show-ui")]
    pub fn insert(
        &self,
        id: &str,
        call: &crate::show_ui::PreparedCall,
        version: &occipital_lang::VersionDescriptor,
        fallback: &occipital_lang::FallbackText,
    ) -> Result<(), StoreError> {
        self.sqlite.with_conn_fenced(|tx| {
            let created_at_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|time| i64::try_from(time.as_millis()).ok())
                .ok_or(rusqlite::Error::InvalidQuery)?;
            tx.execute(
                "INSERT INTO native_records (
                    record_id, owner, ref, scope_epoch, principal, created_at_ms,
                    title, summary, source, source_hash, data, data_format, data_columns,
                    output_refs, execution_dialect, catalog_version, catalog_digest,
                    catalog_layout, fallback, fallback_truncated, now_ms, zone
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL, NULL, NULL,
                    ?11, ?12, ?13, ?14, ?15, ?16, ?17, 'UTC'
                )",
                rusqlite::params![
                    id,
                    principal_text(&call.provenance.owner),
                    call.provenance.scope_ref,
                    call.provenance.scope_epoch,
                    principal_text(&call.provenance.principal),
                    created_at_ms,
                    call.context.show_ui.title,
                    call.context.show_ui.summary,
                    call.source.as_bytes(),
                    source_hash(call.source.as_bytes()),
                    version.execution_dialect,
                    version.catalog.version,
                    version.catalog.digest,
                    version.catalog.layout,
                    fallback.text.as_str(),
                    fallback.truncated,
                    call.now_ms,
                ],
            )?;
            Ok(())
        })
    }
}

pub fn source_hash(source: &[u8]) -> String {
    let digest = hmac_sha256::Hash::hash(source);
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

/// Keep the principal kind with its id; a bare module id would lose the
/// distinction between attested modules and direct or unverified callers.
pub fn principal_text(principal: &subc_protocol::Principal) -> String {
    match principal {
        subc_protocol::Principal::Reserved { module_id } => format!("reserved:{module_id}"),
        subc_protocol::Principal::Direct => "direct".into(),
        subc_protocol::Principal::Unverified => "unverified".into(),
    }
}

fn user_version(conn: &rusqlite::Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

/// Open the store the descriptor names and bring its schema up to
/// [`SCHEMA_VERSION`].
///
/// The version is read before anything is written, so a store from a newer
/// build is refused untouched. Each migration runs in its own fenced write,
/// which rereads the version inside the transaction and sets the new
/// `user_version` in the same commit as the schema change.
pub fn open(descriptor: &StorageDescriptor) -> Result<Store, OpenError> {
    let StorageBackend::Sqlite { path } = &descriptor.backend else {
        return Err(OpenError::Store(StoreError::UnsupportedBackend(
            descriptor.backend.label().to_owned(),
        )));
    };
    let path = std::path::Path::new(path);
    let data_root = path
        .parent()
        .filter(|parent| parent.is_absolute())
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| {
            OpenError::BadDescriptor(format!("store path {} has no folder", path.display()))
        })?;
    let file_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            OpenError::BadDescriptor(format!("store path {} has no file name", path.display()))
        })?;
    let sqlite = open_sqlite(descriptor)?;
    let found = sqlite.with_conn(user_version)?;
    if !(0..=SCHEMA_VERSION).contains(&found) {
        return Err(OpenError::VersionUnsupported { found });
    }
    for (from, statements) in MIGRATIONS.iter().enumerate().skip(found as usize) {
        let from = from as i64;
        sqlite.with_conn_fenced(|tx| {
            // The lease keeps other writers out, so the version cannot have
            // moved since it was read; checking again keeps a migration from
            // ever running twice even if that assumption breaks.
            if user_version(tx)? == from {
                tx.execute_batch(statements)?;
                tx.pragma_update(None, "user_version", from + 1)?;
            }
            Ok(())
        })?;
    }
    let migrated = sqlite.with_conn(user_version)?;
    if migrated != SCHEMA_VERSION {
        return Err(OpenError::VersionUnsupported { found: migrated });
    }
    Ok(Store {
        sqlite: std::sync::Arc::new(sqlite),
        data_root,
        file_name,
    })
}

/// Open the store HELLO_ACK names, or the reason startup must stop.
pub fn open_from_ack(ack: &ModuleHelloAckBody) -> Result<Store, Fatal> {
    descriptor_from_ack(ack)
        .and_then(|descriptor| open(&descriptor))
        .map_err(|error| error.fatal())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use cortexkit_store::Isolation;
    use cortexkit_test_support::ScratchDir;

    pub(crate) fn descriptor(dir: &ScratchDir) -> StorageDescriptor {
        StorageDescriptor {
            module_id: "occipital".into(),
            storage_namespace: "default".into(),
            isolation: Isolation::Module,
            backend: StorageBackend::Sqlite {
                path: dir
                    .path()
                    .join("cortexkit")
                    .join("occipital")
                    .join("store.db")
                    .to_string_lossy()
                    .into_owned(),
            },
        }
    }

    /// A migrated store at `path`, as HELLO_ACK would name it, for the app
    /// reader's table tests.
    pub(crate) fn shared(path: &std::path::Path) -> std::sync::Arc<SqliteStore> {
        let descriptor = StorageDescriptor {
            module_id: "occipital".into(),
            storage_namespace: "default".into(),
            isolation: Isolation::Module,
            backend: StorageBackend::Sqlite {
                path: path.to_string_lossy().into_owned(),
            },
        };
        open(&descriptor).unwrap().shared()
    }

    fn ack_with(descriptor: &StorageDescriptor) -> ModuleHelloAckBody {
        ModuleHelloAckBody {
            negotiated_ver: subc_protocol::PROTOCOL_VERSION,
            subc_ops: vec![],
            subc_capabilities: vec![],
            storage: Some(serde_json::to_value(descriptor).unwrap()),
            machine_id: None,
        }
    }

    fn db_path(descriptor: &StorageDescriptor) -> String {
        match &descriptor.backend {
            StorageBackend::Sqlite { path } => path.clone(),
            other => panic!("not sqlite: {other:?}"),
        }
    }

    /// `(name, declared type, NOT NULL, primary key position)` for each
    /// column, written out from the schema the store is specified to have,
    /// not derived from the DDL the code runs.
    const NATIVE_RECORDS_COLUMNS: &[(&str, &str, bool, i64)] = &[
        ("record_id", "TEXT", true, 1),
        ("owner", "TEXT", true, 0),
        ("ref", "TEXT", true, 0),
        ("scope_epoch", "INTEGER", true, 0),
        ("principal", "TEXT", true, 0),
        ("created_at_ms", "INTEGER", true, 0),
        ("title", "TEXT", true, 0),
        ("summary", "TEXT", true, 0),
        ("source", "BLOB", true, 0),
        ("source_hash", "TEXT", true, 0),
        ("data", "TEXT", false, 0),
        ("data_format", "TEXT", false, 0),
        ("data_columns", "TEXT", false, 0),
        ("output_refs", "BLOB", false, 0),
        ("execution_dialect", "TEXT", true, 0),
        ("catalog_version", "TEXT", true, 0),
        ("catalog_digest", "TEXT", true, 0),
        ("catalog_layout", "TEXT", true, 0),
        ("fallback", "TEXT", true, 0),
        ("fallback_truncated", "INTEGER", true, 0),
        ("now_ms", "INTEGER", true, 0),
        ("zone", "TEXT", true, 0),
    ];

    fn table_info(store: &Store) -> Vec<(String, String, bool, i64)> {
        store
            .sqlite
            .with_conn(|conn| {
                let mut statement = conn.prepare("PRAGMA table_info(native_records)")?;
                let rows = statement.query_map([], |row| {
                    Ok((
                        row.get::<_, String>("name")?,
                        row.get::<_, String>("type")?,
                        row.get::<_, i64>("notnull")? == 1,
                        row.get::<_, i64>("pk")?,
                    ))
                })?;
                rows.collect()
            })
            .unwrap()
    }

    fn pinned(path: &str) -> Vec<u8> {
        let full = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pinned")
            .join(path);
        std::fs::read(&full).unwrap_or_else(|e| panic!("{}: {e}", full.display()))
    }

    #[test]
    fn the_pinned_hello_ack_sample_decodes_and_names_no_store() {
        let bytes = pinned("subc-protocol-0.33.1/module_hello_ack_body.json");
        let ack: ModuleHelloAckBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(ack.negotiated_ver, 2);
        assert_eq!(ack.storage, None);
        // The sample is a daemon with no managed storage: startup stops.
        let fatal = open_from_ack(&ack).err().unwrap();
        assert_eq!(fatal.code, 1);
        assert!(fatal.line.starts_with(UNAVAILABLE_PREFIX), "{}", fatal.line);
    }

    #[test]
    fn each_pinned_sqlite_descriptor_decodes_from_hello_ack_and_opens() {
        let vectors: serde_json::Value =
            serde_json::from_slice(&pinned("cortexkit-store-types-0.2.2/storage_vectors.json"))
                .unwrap();
        let data_home = vectors["data_home"].as_str().unwrap();
        let ack_sample: serde_json::Value =
            serde_json::from_slice(&pinned("subc-protocol-0.33.1/module_hello_ack_body.json"))
                .unwrap();
        let entries = vectors["vectors"].as_array().unwrap();
        assert!(!entries.is_empty());
        for entry in entries {
            let mut ack = ack_sample.clone();
            ack["storage"] = entry["sqlite_descriptor"].clone();
            let ack: ModuleHelloAckBody =
                serde_json::from_slice(&serde_json::to_vec(&ack).unwrap()).unwrap();
            let descriptor = descriptor_from_ack(&ack).unwrap();
            assert_eq!(descriptor.module_id, entry["module_id"].as_str().unwrap());
            assert_eq!(descriptor.storage_namespace, "default");
            assert_eq!(descriptor.isolation, cortexkit_store::Isolation::Module);
            let path = db_path(&descriptor);
            assert_eq!(path, entry["sqlite_store_path"].as_str().unwrap());

            // The same descriptor, moved under a scratch data home, opens
            // and migrates through the production path.
            let dir = ScratchDir::new("occipital-store-vector");
            let mut local = descriptor.clone();
            local.backend = StorageBackend::Sqlite {
                path: path.replacen(data_home, &dir.path().to_string_lossy(), 1),
            };
            let store = open(&local).unwrap();
            assert_eq!(store.sqlite.with_conn(user_version).unwrap(), 1);
        }
        // Occipital's own store sits where the daemon's rule puts it.
        assert_eq!(
            cortexkit_store::sqlite_store_path(data_home, "occipital"),
            format!("{data_home}/cortexkit/occipital/store.db")
        );
    }

    #[test]
    fn a_new_store_has_native_records_column_for_column_at_version_1() {
        let dir = ScratchDir::new("occipital-store-new");
        let store = open(&descriptor(&dir)).unwrap();
        let expected: Vec<(String, String, bool, i64)> = NATIVE_RECORDS_COLUMNS
            .iter()
            .map(|(name, ty, not_null, pk)| ((*name).into(), (*ty).into(), *not_null, *pk))
            .collect();
        assert_eq!(table_info(&store), expected);
        assert_eq!(store.sqlite.with_conn(user_version).unwrap(), 1);
        assert_eq!(SCHEMA_VERSION, 1);
    }

    #[test]
    fn reopening_a_version_1_store_keeps_its_rows_and_version() {
        let dir = ScratchDir::new("occipital-store-reopen");
        let descriptor = descriptor(&dir);
        {
            let store = open(&descriptor).unwrap();
            store
                .sqlite
                .with_conn_fenced(|tx| {
                    tx.execute(
                        "INSERT INTO native_records VALUES ('r_1','o','f',1,'p',2,'t','s',X'00',\
                         'sha256:x',NULL,NULL,NULL,NULL,'1.0','v','d','l','fb',0,3,'UTC')",
                        [],
                    )
                })
                .unwrap();
        }
        let store = open(&descriptor).unwrap();
        let count: i64 = store
            .sqlite
            .with_conn(|c| c.query_row("SELECT COUNT(*) FROM native_records", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(store.sqlite.with_conn(user_version).unwrap(), 1);
    }

    #[test]
    fn a_version_2_store_stops_startup_with_exit_1_and_is_left_untouched() {
        let dir = ScratchDir::new("occipital-store-newer");
        let descriptor = descriptor(&dir);
        drop(open(&descriptor).unwrap());
        {
            let conn = rusqlite::Connection::open(db_path(&descriptor)).unwrap();
            conn.pragma_update(None, "user_version", 2).unwrap();
            conn.execute_batch("CREATE TABLE from_a_newer_build (x)")
                .unwrap();
        }
        let fatal = match open_from_ack(&ack_with(&descriptor)) {
            Ok(_) => panic!("a version 2 store was served"),
            Err(fatal) => fatal,
        };
        assert_eq!(fatal.code, 1);
        assert!(
            fatal.line.starts_with(VERSION_UNSUPPORTED_PREFIX),
            "{}",
            fatal.line
        );
        assert!(fatal.line.contains("user_version 2"), "{}", fatal.line);
        let conn = rusqlite::Connection::open(db_path(&descriptor)).unwrap();
        assert_eq!(user_version(&conn).unwrap(), 2);
    }

    #[test]
    fn hello_ack_without_storage_stops_startup_with_exit_1() {
        let mut ack = ack_with(&descriptor(&ScratchDir::new("occipital-store-none")));
        ack.storage = None;
        let fatal = open_from_ack(&ack).err().unwrap();
        assert_eq!(fatal.code, 1);
        assert!(fatal.line.starts_with(UNAVAILABLE_PREFIX), "{}", fatal.line);
    }

    #[test]
    fn a_postgres_descriptor_is_refused_before_touching_anything() {
        let mut descriptor = descriptor(&ScratchDir::new("occipital-store-pg"));
        descriptor.backend = StorageBackend::Postgres {
            dsn: "postgres://x".into(),
            database: "x".into(),
        };
        let error = open(&descriptor).err().unwrap();
        assert!(matches!(
            error,
            OpenError::Store(StoreError::UnsupportedBackend(_))
        ));
        assert_eq!(error.fatal().code, 1);
    }
}
