//! `occipital-check` is the gate tool providers run on their templates before
//! shipping them, so it asks for the tool catalog's current digest even though
//! Occipital still draws a template naming an earlier one.

use std::process::Command;

use occipital_registry::view::tool_catalog_digest;
use serde_json::json;

const ROW: &str = "<Card appearance=\"clear\" direction=\"row\" gap=\"s\">\n<InlineCode code={input.command} />\n<Slot width=\"narrow\" needs={view.record}>\n<Badge text={view.record.exitCode == 0 ? \"Done\" : \"Failed\"} />\n</Slot>\n</Card>\n";

/// Runs `occipital-check` on a view whose row names `digest`, returning its
/// exit code and what it printed.
fn check(digest: &str) -> (i32, String) {
    let file = std::env::temp_dir().join(format!(
        "occipital-check-{}-{}.json",
        std::process::id(),
        &digest[digest.len() - 8..]
    ));
    let document = json!({
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
    });
    std::fs::write(&file, document.to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_occipital-check"))
        .arg(&file)
        .output()
        .unwrap();
    std::fs::remove_file(&file).unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

#[test]
fn a_template_naming_the_current_digest_passes() {
    let (code, printed) = check(&tool_catalog_digest());
    assert_eq!(code, 0, "{printed}");
    assert!(printed.contains(": row: passed (sha256:"), "{printed}");
}

#[test]
fn a_template_naming_an_earlier_digest_is_asked_for_the_current_one() {
    let earlier = "sha256:9a23139f4f1f48d6ed366a4a48b9221b1e5ac0ccd87473c0b241b8a37a876fbf";
    let (code, printed) = check(earlier);
    assert_eq!(code, 1, "{printed}");
    assert!(
        printed.contains(&format!(
            ": row: it names an earlier tool catalog digest; name {}",
            tool_catalog_digest()
        )),
        "{printed}"
    );
}
