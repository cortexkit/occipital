#![cfg(feature = "show-ui")]

#[path = "../examples/support/mod.rs"]
mod support;

use cortexkit_role_tool_provider::result::{validate, ToolResult};
use cortexkit_store::StorageBackend;
use cortexkit_test_support::ScratchDir;
use occipital_module::{
    native::{envelope, Boundary, Executor, Language},
    serve::NativeExecutor,
    store,
};
use serde_json::{json, Value};
use support::{call, decode, descriptor, Stub, SAMPLE_ID};

fn connection(dir: &ScratchDir) -> rusqlite::Connection {
    let StorageBackend::Sqlite { path } = descriptor(dir).backend else {
        panic!("sqlite");
    };
    rusqlite::Connection::open(path).unwrap()
}

fn rows(dir: &ScratchDir) -> i64 {
    connection(dir)
        .query_row("SELECT count(*) FROM native_records", [], |row| row.get(0))
        .unwrap()
}

fn run(name: &'static str, db: &store::Store) -> (Value, usize) {
    decode(Executor(Stub(name)).execute(call().unwrap(), Some(db))).unwrap()
}

fn assert_result(result: &Value, outcome: &str, code: Option<&str>) {
    let content = &result["structuredContent"];
    assert_eq!(content["schema"], "occipital.show_ui/1");
    assert_eq!(content["outcome"], outcome);
    assert_eq!(result["isError"], outcome != "stored");
    let expected_keys = match outcome {
        "stored" => vec!["check", "kind", "outcome", "recordId", "schema", "version"],
        "check_failed" => vec!["check", "kind", "outcome", "schema"],
        _ => vec!["code", "message", "outcome", "schema"],
    };
    assert_eq!(
        content
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        expected_keys
    );
    if matches!(outcome, "stored" | "check_failed") {
        assert_eq!(content["kind"], "native");
        assert_eq!(content["check"]["ok"], outcome == "stored");
        let text = result["content"][0]["text"].as_str().unwrap();
        let (_, check_text) = text.split_once('\n').unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(check_text).unwrap(),
            content["check"]
        );
    }
    match code {
        Some(code) => {
            assert_eq!(result["errorCode"], code);
            if outcome != "check_failed" {
                assert_eq!(content["code"], code);
            }
        }
        None => assert!(result.get("errorCode").is_none()),
    }
    validate(&serde_json::from_value::<ToolResult>(result.clone()).unwrap()).unwrap();
}

#[test]
fn outcomes_and_host_error_codes_are_literal_and_store_only_on_ok() {
    let dir = ScratchDir::new("occipital-outcomes");
    let db = store::open(&descriptor(&dir)).unwrap();
    for (stub, outcome, code) in [
        ("error", "check_failed", "check_failed"),
        ("abort", "check_failed", "check_failed"),
        ("rejected", "failed", "check_refused"),
        ("refused", "failed", "check_refused"),
        ("invalid_host", "failed", "invalid_host_value"),
        ("internal", "failed", "internal"),
    ] {
        let before = rows(&dir);
        let (result, _) = run(stub, &db);
        assert_result(&result, outcome, Some(code));
        if stub == "abort" {
            assert_eq!(
                result["structuredContent"]["check"]["trial"]["outcome"],
                "E_FUEL"
            );
            assert_eq!(
                result["structuredContent"]["check"]["diagnostics"],
                json!([])
            );
        }
        assert_eq!(rows(&dir), before, "{stub}");
        let (next, _) = decode(Executor(Language).execute(call().unwrap(), Some(&db))).unwrap();
        assert_result(&next, "stored", None);
        assert_eq!(rows(&dir), before + 1);
    }
    let before = rows(&dir);
    let (result, _) = run("ok", &db);
    assert_result(&result, "stored", None);
    assert_eq!(rows(&dir), before + 1);
}

#[test]
fn panic_at_each_boundary_call_is_internal_and_executor_keeps_serving() {
    struct PanicOnce {
        stage: &'static str,
        panicked: std::sync::atomic::AtomicBool,
    }
    impl PanicOnce {
        fn fail_at(&self, stage: &str) {
            if self.stage == stage
                && !self
                    .panicked
                    .swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                panic!("stub panic at {stage}");
            }
        }
    }
    impl Boundary for PanicOnce {
        fn check(
            &self,
            call: &occipital_module::show_ui::PreparedCall,
        ) -> Result<occipital_lang::CheckResult, occipital_lang::HostError> {
            self.fail_at("check");
            Language.check(call)
        }
        fn fallback(
            &self,
            call: &occipital_module::show_ui::PreparedCall,
        ) -> occipital_lang::FallbackText {
            self.fail_at("fallback");
            Language.fallback(call)
        }
        fn version(&self) -> occipital_lang::VersionDescriptor {
            self.fail_at("version");
            Language.version()
        }
    }
    let dir = ScratchDir::new("occipital-panics");
    let db = store::open(&descriptor(&dir)).unwrap();
    for (index, name) in ["check", "fallback", "version"].into_iter().enumerate() {
        let executor = Executor(PanicOnce {
            stage: name,
            panicked: false.into(),
        });
        let (result, _) = decode(executor.execute(call().unwrap(), Some(&db))).unwrap();
        assert_result(&result, "failed", Some("internal"));
        assert_eq!(rows(&dir), index as i64);
        let (next, _) = decode(executor.execute(call().unwrap(), Some(&db))).unwrap();
        assert_result(&next, "stored", None);
        assert_eq!(rows(&dir), index as i64 + 1);
    }
}

#[test]
fn exact_utf8_envelope_cap_is_checked_before_commit() {
    let dir = ScratchDir::new("occipital-result-cap");
    let db = store::open(&descriptor(&dir)).unwrap();
    for (stub, target) in [("cap_at", 32_768), ("cap_over", 32_769)] {
        let stub = Stub(stub);
        let version = stub.version();
        assert!(version.catalog.version.contains('雪'));
        let candidate = json!({"schema":"occipital.show_ui/1", "outcome":"stored", "kind":"native",
            "recordId":SAMPLE_ID, "check":stub.result(), "version":version});
        assert_eq!(envelope(candidate, None).unwrap().len(), target);
    }
    let (over, _) = run("cap_over", &db);
    assert_result(&over, "failed", Some("result_too_large"));
    assert_eq!(rows(&dir), 0);
    let (at, bytes) = run("cap_at", &db);
    assert_result(&at, "stored", None);
    assert_eq!(bytes, 32_768);
    assert_eq!(rows(&dir), 1);
    let (real, bytes) = decode(Executor(Language).execute(call().unwrap(), Some(&db))).unwrap();
    assert_result(&real, "stored", None);
    assert_eq!(real["structuredContent"]["check"]["diagnostics"], json!([]));
    assert!(bytes < 32_768);
}

#[test]
fn trimming_removes_warnings_then_errors_last_first_without_changing_outcome() {
    let dir = ScratchDir::new("occipital-trimming");
    let db = store::open(&descriptor(&dir)).unwrap();
    for (stub, outcome, omitted, retained, original) in [
        (
            "trim_errors",
            "check_failed",
            3,
            vec!["first-error", "middle-error"],
            5,
        ),
        (
            "trim_warnings",
            "stored",
            1,
            vec!["first-warning", "middle-warning"],
            3,
        ),
    ] {
        let (result, bytes) = run(stub, &db);
        assert_result(
            &result,
            outcome,
            if outcome == "stored" {
                None
            } else {
                Some("check_failed")
            },
        );
        assert!(bytes <= 32_768);
        let check = &result["structuredContent"]["check"];
        assert_eq!(check["omitted"], omitted);
        let diagnostics = check["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), retained.len());
        for (diagnostic, prefix) in diagnostics.iter().zip(retained) {
            assert!(diagnostic["subject"].as_str().unwrap().starts_with(prefix));
        }
        let untrimmed = Stub(stub).result();
        assert_eq!(untrimmed.omitted(), 0);
        assert_eq!(untrimmed.diagnostics().len(), original);
    }
    assert_eq!(rows(&dir), 1);
}

#[test]
fn failed_fenced_insert_is_store_failed_and_leaves_zero_rows() {
    let dir = ScratchDir::new("occipital-write-failed");
    let db = store::open(&descriptor(&dir)).unwrap();
    connection(&dir).execute_batch("CREATE TRIGGER reject_native BEFORE INSERT ON native_records BEGIN SELECT RAISE(ABORT, 'test write refusal'); END;").unwrap();
    let (result, _) = run("ok", &db);
    assert_result(&result, "failed", Some("store_failed"));
    assert_eq!(rows(&dir), 0);
    connection(&dir)
        .execute_batch("DROP TRIGGER reject_native")
        .unwrap();
    let (result, _) = run("ok", &db);
    assert_result(&result, "stored", None);
    assert_eq!(rows(&dir), 1);
}

#[test]
fn reopened_row_preserves_every_phase_one_column_and_optional_columns_are_null() {
    let dir = ScratchDir::new("occipital-row-roundtrip");
    let descriptor = descriptor(&dir);
    let db = store::open(&descriptor).unwrap();
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let (result, _) = decode(Executor(Language).execute(call().unwrap(), Some(&db))).unwrap();
    assert_result(&result, "stored", None);
    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let content = &result["structuredContent"];
    let id = content["recordId"].as_str().unwrap();
    assert_eq!(id.len(), 34);
    assert!(id.starts_with("r_"));
    assert!(id[2..]
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    drop(db);
    let reopened = store::open(&descriptor).unwrap();
    let conn = connection(&dir);
    let row: Vec<Value> = conn
        .query_row("SELECT * FROM native_records", [], |row| {
            (0..22)
                .map(|i| {
                    Ok(match row.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => Value::Null,
                        rusqlite::types::ValueRef::Integer(value) => json!(value),
                        rusqlite::types::ValueRef::Text(value) => {
                            json!(std::str::from_utf8(value).unwrap())
                        }
                        rusqlite::types::ValueRef::Blob(value) => json!(value),
                        _ => panic!("unexpected SQLite value"),
                    })
                })
                .collect()
        })
        .unwrap();
    let created = row[5].as_i64().unwrap();
    assert!((before..=after).contains(&created));
    let version = &content["version"];
    assert_eq!(
        row,
        vec![
            json!(id),
            json!("reserved:prefrontal-core"),
            json!("head-1"),
            json!(3),
            json!("reserved:broca"),
            json!(created),
            json!("Title 🦉"),
            json!("Summary 雪"),
            json!(support::SOURCE.as_bytes()),
            json!("sha256:ed071d799deb71974a7b079b7d020d467ce7f667114b2bd4618b04e07c9d6769"),
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            version["executionDialect"].clone(),
            version["catalog"]["version"].clone(),
            version["catalog"]["digest"].clone(),
            version["catalog"]["layout"].clone(),
            json!(Language.fallback(&call().unwrap()).text.as_str()),
            json!(0),
            json!(1234),
            json!("UTC"),
        ]
    );
    let prepared = call().unwrap();
    assert!(reopened
        .insert(
            id,
            &prepared,
            &Language.version(),
            &Language.fallback(&prepared)
        )
        .is_err());
    assert_eq!(
        rows(&dir),
        1,
        "record-id collision cannot replace the existing row"
    );
}

#[test]
fn generated_samples_match_committed_bytes_and_literal_outcomes() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors/show-ui-v1/outcomes.json");
    let committed = std::fs::read_to_string(path).unwrap();
    let generated = support::samples().unwrap();
    assert_eq!(
        committed,
        format!("{}\n", serde_json::to_string_pretty(&generated).unwrap())
    );
    let expected = [
        ("stored", "stored", None),
        ("check_failed", "check_failed", Some("check_failed")),
        ("abort", "check_failed", Some("check_failed")),
        ("error", "check_failed", Some("check_failed")),
        ("rejected", "failed", Some("check_refused")),
        ("refused", "failed", Some("check_refused")),
        ("invalid_host", "failed", Some("invalid_host_value")),
        ("internal", "failed", Some("internal")),
        ("cap_at", "stored", None),
        ("cap_over", "failed", Some("result_too_large")),
        ("oversize_candidate", "stored", None),
        ("trim_errors", "check_failed", Some("check_failed")),
        ("trim_warnings", "stored", None),
        ("store_failed", "failed", Some("store_failed")),
        ("html", "refused", Some("kind_unsupported")),
        ("unscoped", "refused", Some("route_unscoped")),
        ("unknown", "refused", Some("unknown_argument")),
        ("missing", "refused", Some("argument_missing")),
        ("invalid", "refused", Some("argument_invalid")),
        ("forbidden", "refused", Some("argument_forbidden")),
        (
            "references-unavailable",
            "refused",
            Some("references_unavailable"),
        ),
        ("data-unknown", "refused", Some("data_reference_unknown")),
        ("malformed-descriptor", "refused", Some("argument_invalid")),
    ];
    let samples: Value = serde_json::from_str(&committed).unwrap();
    let samples = samples.as_array().unwrap();
    assert_eq!(samples.len(), expected.len());
    for (sample, (name, outcome, code)) in samples.iter().zip(expected) {
        assert_eq!(sample["name"], name);
        let result = if name == "oversize_candidate" {
            &sample["candidate"]
        } else {
            &sample["response"]
        };
        assert_result(result, outcome, code);
        if name == "cap_at" {
            assert_eq!(sample["envelopeBytes"], 32_768);
        }
        if name == "oversize_candidate" {
            assert_eq!(sample["envelopeBytes"], 32_769);
        }
    }
}

#[test]
fn persisted_version_and_fallback_come_from_the_same_boundary_values_as_the_reply() {
    let dir = ScratchDir::new("occipital-boundary-metadata");
    let db = store::open(&descriptor(&dir)).unwrap();
    let (result, _) = run("metadata", &db);
    assert_result(&result, "stored", None);
    assert_eq!(
        result["structuredContent"]["version"],
        json!({"executionDialect":"9.8", "catalog":{
            "version":"7.6", "digest":"abababababababababababababababababababababababababababababababab", "layout":"5",
            "aliases":["cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"]
        }})
    );
    drop(db);
    let _reopened = store::open(&descriptor(&dir)).unwrap();
    let metadata: (String, String, String, String, String, i64) = connection(&dir).query_row(
        "SELECT execution_dialect, catalog_version, catalog_digest, catalog_layout, fallback, fallback_truncated FROM native_records", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).unwrap();
    assert_eq!(
        metadata,
        (
            "9.8".into(),
            "7.6".into(),
            "ab".repeat(32),
            "5".into(),
            "Fallback 雪".into(),
            1
        )
    );
    assert_eq!(
        store::principal_text(&subc_protocol::Principal::Direct),
        "direct"
    );
    assert_eq!(
        store::principal_text(&subc_protocol::Principal::Unverified),
        "unverified"
    );
}

#[test]
fn superseded_store_fence_is_store_failed_and_leaves_zero_rows() {
    let dir = ScratchDir::new("occipital-fenced-out");
    let db = store::open(&descriptor(&dir)).unwrap();
    connection(&dir)
        .execute(
            "UPDATE cortexkit_fence SET epoch = ?1 WHERE id = 0",
            [db.epoch() + 1],
        )
        .unwrap();
    let (result, _) = run("ok", &db);
    assert_result(&result, "failed", Some("store_failed"));
    assert_eq!(rows(&dir), 0);
}
