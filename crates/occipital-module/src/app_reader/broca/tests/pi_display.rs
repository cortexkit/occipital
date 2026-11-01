use super::*;

const FIXTURES: [&str; 2] = [
    include_str!("../../../../tests/fixtures/pi/pi-1.3.0.jsonl"),
    include_str!("../../../../tests/fixtures/pi/pi-1.4.0.jsonl"),
];

fn pi_source() -> Arc<BrocaSource> {
    let mut binding = resolved();
    binding.reference.runner = "thalamus-pi".into();
    binding.harness = "pi".into();
    BrocaSource::new(binding, FakeRoute::new())
}

fn entries(fixture: &str) -> Vec<Value> {
    fixture
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|row| matches!(row["type"].as_str(), Some("message" | "custom_message")))
        .enumerate()
        .map(|(ordinal, message)| {
            json!({"mid":message["id"], "ordinal":ordinal, "written_at":1234, "message":message})
        })
        .collect()
}

fn fixture_entry(entries: &[Value], mid: &str) -> Value {
    entries
        .iter()
        .find(|entry| entry["mid"] == mid)
        .unwrap()
        .clone()
}

#[test]
fn pi_fixtures_use_app_limits_for_arguments_and_success_and_error_results() {
    let source = pi_source();
    assert_eq!(source.harness(), "pi");
    for fixture in FIXTURES {
        let entries = entries(fixture);
        let rows = source.rows(&entries, false, &[]).unwrap();
        assert_eq!(rows.len(), 18);
        assert_eq!(rows[0].mid, "entry-003");
        assert_eq!(rows[0].ordinal, Some(0));
        assert_eq!(rows[0].written_at, Some(1234));
        assert_eq!(rows[4].display["parts"][0]["type"], "reasoning");
        assert_eq!(
            rows[4].display["parts"][1]["text"],
            "Fixture assistant text."
        );
        assert_eq!(rows[4].display["parts"][2]["name"], "head_fixture_image");
        assert_eq!(
            rows[5].display["parts"][0]["content"],
            "Fixture tool result."
        );
        assert_eq!(
            rows[5].display["parts"][0]["files"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        // Expand cloned recorded payloads, never the checksum-pinned journals.
        for size in [2_047, 2_048, 2_049] {
            let mut call = fixture_entry(&entries, "entry-008");
            let input = "x".repeat(size - 11); // {"text":""} adds eleven JSON bytes.
            call["message"]["message"]["content"][2]["arguments"] = json!({"text":input});
            let rows = source.rows(&[call.clone()], false, &[]).unwrap();
            let part = &rows[0].display["parts"][2];
            assert_eq!(part["arguments"].as_str().unwrap().len(), size.min(2_048));
            assert_eq!(
                part["arguments_truncated"].as_bool().unwrap_or(false),
                size > 2_048
            );
            assert_eq!(part["tool_call_id"], "call_head_fixture");
            let full = source.rows(&[call], true, &[]).unwrap();
            assert_eq!(
                full[0].display["parts"][2]["arguments"]
                    .as_str()
                    .unwrap()
                    .len(),
                size
            );
        }
        for (is_error, cap) in [(false, 16_384), (true, 65_536)] {
            for size in [cap - 1, cap, cap + 1] {
                let mut result = fixture_entry(&entries, "entry-009");
                result["message"]["message"]["content"][0]["text"] = json!("x".repeat(size));
                result["message"]["message"]["isError"] = json!(is_error);
                let rows = source.rows(&[result.clone()], false, &[]).unwrap();
                let part = &rows[0].display["parts"][0];
                assert_eq!(part["content"], "x".repeat(size.min(cap)));
                assert_eq!(part["truncated"].as_bool().unwrap_or(false), size > cap);
                assert_eq!(part["is_error"].as_bool().unwrap_or(false), is_error);
                assert_eq!(part["files"].as_array().unwrap().len(), 1);
                let full = source.rows(&[result], true, &[]).unwrap();
                assert_eq!(full[0].display["parts"][0]["content"], "x".repeat(size));
            }
        }
    }
}

#[test]
fn pi_shell_and_custom_messages_are_not_the_persons_turn() {
    let source = pi_source();
    for fixture in FIXTURES {
        let entries = entries(fixture);
        for full in [false, true] {
            for (mid, role, text) in [
                ("entry-011", "custom", "Fixture custom message."),
                ("entry-012", "bashExecution", "FIXTURE_BASH"),
                ("entry-013", "bashExecution", "FIXTURE_EXCLUDED"),
            ] {
                let rows = source
                    .rows(&[fixture_entry(&entries, mid)], full, &[])
                    .unwrap();
                let display = &rows[0].display;
                assert_ne!(display["role"], "user", "{mid} is not the person's turn");
                assert_eq!(display["role"], role);
                assert!(display["text"].as_str().unwrap().contains(text));
                if role == "bashExecution" {
                    assert_eq!(
                        display["text"],
                        format!("Ran `printf {text}`\n```\n{text}\n```")
                    );
                }
            }
            let user = source
                .rows(&[fixture_entry(&entries, "entry-004")], full, &[])
                .unwrap();
            assert_eq!(user[0].display["role"], "user");
            assert_eq!(user[0].display["text"], "STRING_USER");
        }
    }

    // The recordings have no branch summary; exercise its distinct runtime role too.
    let branch = json!({"mid":"branch", "message":{
        "type":"branch_summary", "summary":"Saved branch summary", "fromId":"entry-005"
    }});
    let rows = source.rows(&[branch], false, &[]).unwrap();
    assert_eq!(rows[0].display["role"], "branchSummary");
    assert!(rows[0].display["text"]
        .as_str()
        .unwrap()
        .contains("Saved branch summary"));
}

#[test]
fn non_pi_harnesses_keep_their_existing_row_decoders() {
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    let entries = [entry("broca", Some(1), 1234)];
    assert_eq!(source.harness(), "broca");
    assert_eq!(
        source.rows(&entries, false, &[]).unwrap()[0].display,
        broca_entries::rows(&entries, false).unwrap()[0].display
    );

    let mut binding = resolved();
    binding.reference.runner = crate::app_reader::resolve::OPENCODE_RUNNER_MODULE_ID.into();
    binding.harness = "opencode".into();
    let source = BrocaSource::new(binding, FakeRoute::new());
    let entries = [json!({"mid":"opencode", "message":{
        "info":{"role":"assistant"}, "parts":[{"type":"text", "text":"OpenCode text"}]
    }})];
    assert_eq!(source.harness(), "opencode");
    assert_eq!(
        source.rows(&entries, false, &[]).unwrap()[0].display,
        occipital_app_reader::opencode_entries::rows(&entries, false).unwrap()[0].display
    );
}
