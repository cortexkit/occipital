use super::*;
use cortexkit_test_support::ScratchDir;

/// The four files, opened as at startup, with every table a move copies.
pub(crate) struct Opened {
    pub main: crate::store::Store,
    pub shared: std::sync::Arc<SqliteStore>,
    pub display: std::sync::Arc<crate::display::store::DisplayStore>,
    pub notices: std::sync::Arc<crate::notices::NoticeStore>,
}

pub(crate) fn opened(dir: &ScratchDir) -> Opened {
    let descriptor = crate::store::tests::descriptor(dir);
    let main = crate::store::open(&descriptor).unwrap();
    crate::app_reader::send_store::Receipts::open(main.shared()).unwrap();
    let display = crate::display::store::DisplayStore::open(&descriptor, main.data_root()).unwrap();
    let notices = crate::notices::NoticeStore::open(&descriptor, main.data_root()).unwrap();
    Opened {
        shared: main.shared(),
        main,
        display: std::sync::Arc::new(display),
        notices: std::sync::Arc::new(notices),
    }
}

impl Opened {
    pub(crate) fn files(&self) -> Files<'_> {
        let (records, details) = self.display.files();
        Files {
            records,
            details,
            notices: self.notices.file(),
            store: &self.shared,
        }
    }
}

fn runner_json(runner: &str) -> String {
    crate::display::store::principal_json(&Principal::Reserved {
        module_id: runner.into(),
    })
    .unwrap()
}

/// One row in every table for `session`, with a call minted by `runner`.
pub(crate) fn seed(files: &Files<'_>, session: &str, runner: &str, call: &str) {
    let write = |file: &SqliteStore, sql: &str, values: Vec<Sql>| {
        file.with_conn_fenced(|tx| tx.execute(sql, rusqlite::params_from_iter(values)))
            .unwrap();
    };
    let text = |s: &str| Sql::Text(s.into());
    write(
        files.records,
        "INSERT INTO display_call_pins (runner, call_key, session, pin, seen_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
        vec![text(runner), text(call), text(session), text("tp1:bash:0123:1"), Sql::Integer(1)],
    );
    for (file, key) in [
        (files.records, format!("{call}-record")),
        (files.details, format!("{call}-detail")),
    ] {
        write(
            file,
            "INSERT INTO display_writes (key, writer, runner, call_key, kind, schema_pin, body, stored_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            vec![
                Sql::Blob(key.into_bytes()),
                text(&runner_json("aft")),
                text(&runner_json(runner)),
                text(call),
                text("record"),
                text("tp1:bash:0123:1"),
                Sql::Blob(b"{\"ok\":true}".to_vec()),
                Sql::Integer(2),
            ],
        );
    }
    write(
        files.notices,
        "INSERT INTO notices (runner, call_key, sender, send_id, session, text, delivered_as, speaks_for, position, seen_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        vec![text(runner), text(call), text("prefrontal"), text(&format!("{session}-ride")), text(session), text("note"), text("tool_result"), Sql::Null, Sql::Integer(0), Sql::Integer(3)],
    );
    write(
        files.notices,
        "INSERT INTO wake_records (session, sender, send_id, runner, body, written_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        vec![text(session), text("prefrontal"), text(&format!("{session}-wake")), text(runner), Sql::Blob(b"{}".to_vec()), Sql::Integer(4)],
    );
    write(
        files.notices,
        "INSERT INTO sends (runner, session, mid, sender, send_id, seen_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        vec![text(runner), text(session), text(&format!("{session}-mid")), text("prefrontal"), text(&format!("{session}-wake")), Sql::Integer(5)],
    );
    write(
        files.store,
        "INSERT INTO harness_app_send_queue (session, client_message_id, harness, send_id, accepted_at, steer_until, state, reason, ended_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        vec![text(session), text(&format!("{session}-msg")), text("broca"), text(&format!("{session}-send")), Sql::Integer(6), Sql::Null, text("unsent"), text("host_refused"), Sql::Integer(7)],
    );
    write(
        files.store,
        "INSERT INTO harness_app_send_wait (session, client_message_id, runner_code, retry_after_ms, cause) VALUES (?1, ?2, ?3, ?4, ?5)",
        vec![text(session), text(&format!("{session}-msg")), text("host_refused"), Sql::Null, text("attachment_unreadable")],
    );
}

pub(crate) fn sessions(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

/// Every row a move would carry for `session`.
pub(crate) fn exported(files: &Files<'_>, session: &str) -> Vec<(String, Vec<Sql>)> {
    let mut rows = Vec::new();
    files
        .export(&sessions(&[session]), |(table, values)| {
            rows.push((table.name.to_owned(), values));
            Ok(())
        })
        .unwrap();
    rows
}

/// The column lists name every column of each table, in a form a move can
/// insert, and the key its primary key. A column a migration adds later
/// fails this until the list carries it, so it is never left behind.
#[test]
fn every_table_lists_all_its_columns_and_its_key() {
    let dir = ScratchDir::new("moves-schema");
    let opened = opened(&dir);
    let files = opened.files();
    for table in TABLES {
        let (columns, key): (Vec<String>, Vec<(i64, String)>) = files
            .file(table.file)
            .with_conn(|conn| {
                let mut statement =
                    conn.prepare(&format!("PRAGMA table_info({})", table.sql_name))?;
                let rows: Vec<(String, i64)> = statement
                    .query_map([], |row| Ok((row.get(1)?, row.get(5)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                let columns = rows.iter().map(|(name, _)| name.clone()).collect();
                let mut key: Vec<(i64, String)> = rows
                    .into_iter()
                    .filter(|(_, pk)| *pk > 0)
                    .map(|(name, pk)| (pk, name))
                    .collect();
                key.sort();
                Ok((columns, key))
            })
            .unwrap();
        let mut listed: Vec<String> = table.columns.iter().map(|c| c.to_string()).collect();
        let mut found = columns.clone();
        listed.sort();
        found.sort();
        assert_eq!(listed, found, "{} columns", table.name);
        let key: Vec<String> = key.into_iter().map(|(_, name)| name).collect();
        assert_eq!(key, table.key, "{} key", table.name);
    }
}

/// A move carries exactly the moved session's rows, its tool records and
/// details chosen through its call pins; the receiver keeps what it already
/// holds; release on the source drops only the moved session's waiting
/// sends, which the receiver follows now.
#[test]
fn a_session_s_rows_export_and_import_whole() {
    let source_dir = ScratchDir::new("moves-source");
    let source = opened(&source_dir);
    let files = source.files();
    seed(&files, "moved", "broca", "moved-call");
    seed(&files, "stays", "broca", "stays-call");

    let mut carried = Vec::new();
    let counts = files
        .export(&sessions(&["moved"]), |row| {
            carried.push(row);
            Ok(())
        })
        .unwrap();
    assert_eq!(counts.values().sum::<u64>(), TABLES.len() as u64);
    assert!(counts.values().all(|n| *n == 1), "{counts:?}");
    for (table, values) in &carried {
        let text = format!("{values:?}");
        assert!(!text.contains("stays"), "{} carried {text}", table.name);
    }

    let receiver_dir = ScratchDir::new("moves-receiver");
    let receiver = opened(&receiver_dir);
    assert_eq!(
        receiver.files().import(&carried).unwrap(),
        TABLES.len() as u64
    );
    // A repeated import keeps every row as it is.
    assert_eq!(receiver.files().import(&carried).unwrap(), 0);
    assert_eq!(
        exported(&receiver.files(), "moved"),
        exported(&files, "moved")
    );

    // Only the moved session's waiting sends leave; its display rows stay.
    assert_eq!(
        files.delete_waiting_sends(&sessions(&["moved"])).unwrap(),
        2
    );
    let left: Vec<String> = exported(&files, "moved")
        .into_iter()
        .map(|(table, _)| table)
        .collect();
    assert_eq!(
        left,
        [
            "call_pins",
            "records",
            "details",
            "notices",
            "wake_records",
            "sends"
        ]
    );
    assert_eq!(exported(&files, "stays").len(), TABLES.len());
}

/// A call minted by another runner and moved into the session (its pin names
/// that runner) still brings its records: they are chosen by the pin's
/// runner, not the session's.
#[test]
fn a_moved_in_call_s_records_follow_its_pin() {
    let dir = ScratchDir::new("moves-carrier");
    let opened = opened(&dir);
    let files = opened.files();
    seed(&files, "moved", "thalamus-harness", "imported-call");
    let rows = exported(&files, "moved");
    let records: Vec<_> = rows
        .iter()
        .filter(|(table, _)| table == "records")
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].1[2], Sql::Text(runner_json("thalamus-harness")));
}
