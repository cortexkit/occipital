//! One row per phone upload: who opened it, where it goes, how much of it is
//! durable, and, once finished, the file it became.

use std::{path::PathBuf, sync::Arc};

use cortexkit_store::{Migration, SqliteStore};
use rusqlite::{params, OptionalExtension};

/// The chain `--migration-chains` reports, so the deploy check sees this table.
pub(crate) const NAMESPACE: &str = "harness_app_upload";
pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    statements: "
CREATE TABLE harness_app_upload (
    upload_id TEXT PRIMARY KEY,
    device TEXT NOT NULL,
    session TEXT NOT NULL,
    project_root TEXT NOT NULL,
    name TEXT NOT NULL,
    mime TEXT NOT NULL,
    size INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    committed INTEGER NOT NULL,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    path TEXT,
    inode INTEGER,
    mtime_ns INTEGER,
    finished_at INTEGER
);
CREATE INDEX harness_app_upload_due ON harness_app_upload(state, expires_at);
CREATE INDEX harness_app_upload_finished ON harness_app_upload(state, finished_at);",
}];

/// Where an upload is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// Taking pieces; its blob holds the committed prefix.
    Open,
    /// Copied into the project; the row keeps the answer a repeated finish gets.
    Finished,
    /// Never finished within its life. The blob is gone and the row stays only so a
    /// late finish is told `upload_expired` instead of `upload_unknown`.
    Expired,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Finished => "finished",
            Self::Expired => "expired",
        }
    }

    fn parse(text: &str) -> rusqlite::Result<Self> {
        match text {
            "open" => Ok(Self::Open),
            "finished" => Ok(Self::Finished),
            "expired" => Ok(Self::Expired),
            other => Err(rusqlite::Error::InvalidColumnType(
                9,
                format!("unknown upload state {other}"),
                rusqlite::types::Type::Text,
            )),
        }
    }
}

/// The file a finished upload became, as it was right after it was written. Cleanup
/// deletes the file only while all three facts still hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placed {
    pub path: PathBuf,
    pub inode: u64,
    pub mtime_ns: i64,
    pub finished_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub upload_id: String,
    /// The opening device's fingerprint; every later call must come from it.
    pub device: String,
    pub session: String,
    pub project_root: PathBuf,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub sha256: String,
    pub committed: u64,
    pub state: State,
    pub created_at: i64,
    pub expires_at: i64,
    pub placed: Option<Placed>,
}

pub(crate) struct Rows(Arc<SqliteStore>);

const COLUMNS: &str =
    "upload_id,device,session,project_root,name,mime,size,sha256,committed,state,\
created_at,expires_at,path,inode,mtime_ns,finished_at";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let path: Option<String> = r.get(12)?;
    let placed = match (path, r.get::<_, Option<i64>>(13)?, r.get(14)?, r.get(15)?) {
        (Some(path), Some(inode), Some(mtime_ns), Some(finished_at)) => Some(Placed {
            path: path.into(),
            // Stored bit for bit: SQLite integers are signed, inode numbers are not.
            inode: inode as u64,
            mtime_ns,
            finished_at,
        }),
        _ => None,
    };
    Ok(Row {
        upload_id: r.get(0)?,
        device: r.get(1)?,
        session: r.get(2)?,
        project_root: r.get::<_, String>(3)?.into(),
        name: r.get(4)?,
        mime: r.get(5)?,
        size: r.get::<_, i64>(6)? as u64,
        sha256: r.get(7)?,
        committed: r.get::<_, i64>(8)? as u64,
        state: State::parse(&r.get::<_, String>(9)?)?,
        created_at: r.get(10)?,
        expires_at: r.get(11)?,
        placed,
    })
}

impl Rows {
    pub fn open(store: Arc<SqliteStore>) -> Result<Self, crate::Error> {
        if store.migrate(NAMESPACE, MIGRATIONS)?.store_ahead() {
            return Err("harness_app_upload_schema_ahead".into());
        }
        Ok(Self(store))
    }

    pub fn insert(&self, row: &Row) -> Result<(), crate::Error> {
        self.0.with_conn(|conn| {
            conn.execute(
                &format!("INSERT INTO harness_app_upload({COLUMNS}) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,NULL,NULL,NULL,NULL)"),
                params![
                    row.upload_id,
                    row.device,
                    row.session,
                    row.project_root.to_string_lossy(),
                    row.name,
                    row.mime,
                    row.size as i64,
                    row.sha256,
                    row.committed as i64,
                    row.state.as_str(),
                    row.created_at,
                    row.expires_at,
                ],
            )
        })?;
        Ok(())
    }

    pub fn get(&self, upload_id: &str) -> Result<Option<Row>, crate::Error> {
        Ok(self.0.with_read(|conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM harness_app_upload WHERE upload_id=?1"),
                [upload_id],
                row,
            )
            .optional()
        })?)
    }

    /// Move the durable prefix. Only an open upload's prefix moves.
    pub fn set_committed(&self, upload_id: &str, committed: u64) -> Result<(), crate::Error> {
        self.0.with_conn(|conn| {
            conn.execute(
                "UPDATE harness_app_upload SET committed=?2 WHERE upload_id=?1 AND state='open'",
                params![upload_id, committed as i64],
            )
        })?;
        Ok(())
    }

    pub fn finish(&self, upload_id: &str, placed: &Placed) -> Result<(), crate::Error> {
        self.0.with_conn(|conn| {
            conn.execute(
                "UPDATE harness_app_upload SET state='finished',path=?2,inode=?3,mtime_ns=?4,finished_at=?5 \
                 WHERE upload_id=?1 AND state='open'",
                params![
                    upload_id,
                    placed.path.to_string_lossy(),
                    placed.inode as i64,
                    placed.mtime_ns,
                    placed.finished_at,
                ],
            )
        })?;
        Ok(())
    }

    pub fn expire(&self, upload_id: &str) -> Result<(), crate::Error> {
        self.0.with_conn(|conn| {
            conn.execute(
                "UPDATE harness_app_upload SET state='expired' WHERE upload_id=?1 AND state='open'",
                [upload_id],
            )
        })?;
        Ok(())
    }

    pub fn delete(&self, upload_id: &str) -> Result<(), crate::Error> {
        self.0.with_conn(|conn| {
            conn.execute(
                "DELETE FROM harness_app_upload WHERE upload_id=?1",
                [upload_id],
            )
        })?;
        Ok(())
    }

    /// Up to `limit` open uploads whose life ended at or before `now`.
    pub fn due_open(&self, now: i64, limit: usize) -> Result<Vec<Row>, crate::Error> {
        self.select(
            "state='open' AND expires_at<=?1 ORDER BY expires_at",
            now,
            limit,
        )
    }

    /// Up to `limit` finished uploads recorded at or before `cutoff`.
    pub fn due_finished(&self, cutoff: i64, limit: usize) -> Result<Vec<Row>, crate::Error> {
        self.select(
            "state='finished' AND finished_at<=?1 ORDER BY finished_at",
            cutoff,
            limit,
        )
    }

    /// Up to `limit` expired uploads opened at or before `cutoff`.
    pub fn due_expired(&self, cutoff: i64, limit: usize) -> Result<Vec<Row>, crate::Error> {
        self.select(
            "state='expired' AND created_at<=?1 ORDER BY created_at",
            cutoff,
            limit,
        )
    }

    fn select(&self, filter: &str, at: i64, limit: usize) -> Result<Vec<Row>, crate::Error> {
        Ok(self.0.with_read(|conn| {
            conn.prepare(&format!(
                "SELECT {COLUMNS} FROM harness_app_upload WHERE {filter} LIMIT ?2"
            ))?
            .query_map(params![at, limit as i64], row)?
            .collect()
        })?)
    }
}
