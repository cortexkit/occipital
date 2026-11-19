//! `occipital-check` is the gate tool providers run on their templates before
//! shipping them: a template must name the tool catalog's current digest.

use std::process::Command;

use occipital_registry::view::tool_catalog_digest;
use serde_json::json;

const ROW: &str = "<Card appearance=\"clear\" direction=\"row\" gap=\"s\">\n<InlineCode code={input.command} />\n<Slot width=\"narrow\" needs={view.record}>\n<Badge text={view.record.exitCode == 0 ? \"Done\" : \"Failed\"} />\n</Slot>\n</Card>\n";

fn fixture(digest: &str) -> serde_json::Value {
    json!({
        "input_schema": {"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]},
        "view": {
            "record_schema": {"name": "aft.bash", "revision": 1, "fields": {"exitCode": "integer"}},
            "reads": [],
            "template": {"row": {
                "schema": "occipital.template/1",
                "profile": "tool_row",
                "executionDialect": "1.0",
                "catalog": {"version": "1.1", "digest": digest},
                "reads": [],
                "source": ROW,
            }},
        },
    })
}

fn run_fixture(label: &str, text: &str) -> (i32, String) {
    let file = std::env::temp_dir().join(format!(
        "occipital-check-{}-{label}.json",
        std::process::id()
    ));
    std::fs::write(&file, text).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_occipital-check"))
        .arg(&file)
        .output()
        .unwrap();
    std::fs::remove_file(&file).unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8([output.stdout, output.stderr].concat()).unwrap(),
    )
}

fn check(digest: &str) -> (i32, String) {
    run_fixture(&digest[digest.len() - 8..], &fixture(digest).to_string())
}

#[test]
fn a_template_naming_the_current_digest_passes() {
    let (code, printed) = check(&tool_catalog_digest());
    assert_eq!(code, 0, "{printed}");
    assert!(printed.contains(": row: passed (sha256:"), "{printed}");
}

/// The digest from before the `narrow` Slot width: retired, since the catalog
/// changes in place until the apps' first release.
#[test]
fn a_template_naming_a_retired_digest_fails() {
    let retired = "sha256:9a23139f4f1f48d6ed366a4a48b9221b1e5ac0ccd87473c0b241b8a37a876fbf";
    let (code, printed) = check(retired);
    assert_eq!(code, 1, "{printed}");
    assert!(!printed.contains(": row: passed"), "{printed}");
    assert!(printed.contains(": row: "), "{printed}");
}

#[test]
fn duplicate_members_fail_at_their_json_pointer() {
    let text = fixture(&tool_catalog_digest()).to_string();
    for (text, pointer) in [
        (
            text.replace(
                r#""profile":"tool_row""#,
                r#""profile":"tool_page","pro\u0066ile":"tool_row""#,
            ),
            "/view/template/row/profile",
        ),
        (
            text.replacen('{', r#"{"samples":[{"x/y~":1,"x/y~":2}],"#, 1),
            "/samples/0/x~1y~0",
        ),
    ] {
        let (code, printed) = run_fixture("duplicate", &text);
        assert_eq!(code, 2, "{printed}");
        assert!(printed.contains(pointer), "{printed}");
    }
    let numbers = text.replacen('{', r#"{"extra":[1.25,3e-2,9007199254740992],"#, 1);
    assert_eq!(run_fixture("ordinary-numbers", &numbers).0, 0);
}

#[test]
fn view_reads_must_be_an_array() {
    let mut doc = fixture(&tool_catalog_digest());
    doc["view"]["reads"] = json!("none");
    let (code, printed) = run_fixture("reads", &doc.to_string());
    assert_ne!(code, 0, "{printed}");
    assert!(printed.contains("reads"));
}

#[test]
fn samples_are_validated_without_a_row() {
    let mut doc = fixture(&tool_catalog_digest());
    doc["view"]["template"] = json!({});
    for member in ["record", "completion"] {
        doc["samples"] = json!([{"input":{}, member:false}]);
        let (code, printed) = run_fixture("sample", &doc.to_string());
        assert_eq!(code, 1, "{printed}");
        assert!(
            printed.contains(&format!("sample's {member} is not an object")),
            "{printed}"
        );
    }
    doc["samples"] = json!([{"input":{}}]);
    let (code, printed) = run_fixture("sample", &doc.to_string());
    assert_eq!(code, 0, "{printed}");
    assert!(printed.contains("not measured"));
}

#[cfg(unix)]
#[test]
fn non_utf8_path_has_a_normal_error_exit() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let output = Command::new(env!("CARGO_BIN_EXE_occipital-check"))
        .arg(OsStr::from_bytes(b"view-\xff.json"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("view-") && error.contains("UTF-8"),
        "{error}"
    );
}
