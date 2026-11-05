//! A message another module relayed into a chat (llm-runner §9.7.1) reaches the
//! apps without the header the runner stored in front of it, and with an
//! `origin` naming that module, on every runner. The header is cut only when
//! the read's `attribution` fact matches the stored text exactly; text that
//! merely looks like a header, or a fact that doesn't match, is shown whole.

use cortexkit_role_llm_runner::attribution;
use occipital_app_reader::broca_entries::{display_rows, rows, DisplayProfile};
use occipital_app_reader::source::Row;
use serde_json::{json, Value};

const MODULE: &str = "wernicke";
const TEXT: &str = "Chat message from @ana in Slack #design: can you look at the login bug?";

/// The stored text: the contract's header for `MODULE`, then `text`.
fn stored(text: &str) -> String {
    format!("{}{text}", attribution::header(MODULE))
}

/// The read's fact for a header at stored part 0.
fn fact(text: &str) -> Value {
    json!(attribution::verified(0, MODULE, &stored(text)).expect("the header verifies"))
}

#[derive(Clone, Copy, Debug)]
enum Runner {
    Broca,
    OpenCode2,
    Pi,
}

const RUNNERS: [Runner; 3] = [Runner::Broca, Runner::OpenCode2, Runner::Pi];

/// One user entry holding `text` as each runner stores it, with `fact` beside it.
fn row(runner: Runner, text: &str, fact: Option<Value>) -> Row {
    let message = match runner {
        Runner::Broca => json!({"role": "user", "content": [{"type": "text", "text": text}]}),
        Runner::OpenCode2 => json!({"type": "user", "text": text, "skills": [], "files": []}),
        Runner::Pi => json!({"type": "message",
            "message": {"role": "user", "content": [{"type": "text", "text": text}]}}),
    };
    let mut entry = json!({"mid": "m1", "ordinal": 1, "written_at": 1_000, "message": message});
    if let Some(fact) = fact {
        entry["attribution"] = fact;
    }
    let entries = [entry];
    let mut rows = match runner {
        Runner::Broca => rows(&entries, false),
        Runner::OpenCode2 => display_rows(&entries, false, DisplayProfile::OpenCode2, &[]),
        Runner::Pi => display_rows(&entries, false, DisplayProfile::Pi, &[]),
    }
    .unwrap();
    assert_eq!(rows.len(), 1);
    rows.remove(0)
}

fn texts(display: &Value) -> Vec<&str> {
    display["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["type"] == "text")
        .map(|part| part["text"].as_str().unwrap())
        .collect()
}

#[test]
fn a_relayed_message_shows_without_its_header_and_names_its_module() {
    for runner in RUNNERS {
        let display = row(runner, &stored(TEXT), Some(fact(TEXT))).display;
        assert_eq!(display["text"], TEXT, "{runner:?}: {display}");
        assert_eq!(texts(&display), [TEXT], "{runner:?}");
        assert_eq!(display["origin"], json!({"module": MODULE}), "{runner:?}");
        assert_eq!(display["parts"][0]["partIndex"], 0, "{runner:?}");
    }
}

#[test]
fn a_header_with_no_text_after_it_leaves_no_text_part() {
    for runner in RUNNERS {
        let display = row(runner, &stored(""), Some(fact(""))).display;
        assert_eq!(display["text"], "", "{runner:?}: {display}");
        assert!(texts(&display).is_empty(), "{runner:?}: {display}");
        assert_eq!(display["origin"], json!({"module": MODULE}), "{runner:?}");
    }
}

#[test]
fn a_header_without_a_matching_fact_is_shown_whole() {
    let mut longer = fact(TEXT);
    longer["header_bytes"] = json!(longer["header_bytes"].as_u64().unwrap() + 1);
    let mut other_module = fact(TEXT);
    other_module["module"] = json!("chat-gateway");
    for runner in RUNNERS {
        for (case, fact) in [
            ("no fact", None),
            ("longer header", Some(longer.clone())),
            ("another module", Some(other_module.clone())),
            ("malformed fact", Some(json!({"part": "0"}))),
        ] {
            let display = row(runner, &stored(TEXT), fact).display;
            assert_eq!(
                display["text"],
                stored(TEXT),
                "{runner:?}, {case}: {display}"
            );
            assert!(display.get("origin").is_none(), "{runner:?}, {case}");
        }
    }
}

#[test]
fn a_fact_on_text_without_the_header_changes_nothing() {
    for runner in RUNNERS {
        let display = row(runner, TEXT, Some(fact(TEXT))).display;
        assert_eq!(display["text"], TEXT, "{runner:?}: {display}");
        assert!(display.get("origin").is_none(), "{runner:?}");
    }
}
