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

/// Pi rows reach the apps in the app row shape, with the apps' own bounds:
/// arguments are cut at 2,048 bytes, and a tool part carries the result's
/// length rather than its text, except on an exact read, which keeps both whole.
#[test]
fn pi_rows_have_the_app_shape_and_app_bounds() {
    let source = pi_source();
    assert_eq!(source.harness(), "pi");
    for fixture in FIXTURES {
        let entries = entries(fixture);
        let rows = source.rows(&entries, false, &[]).unwrap();
        assert_eq!(rows.len(), 18);
        assert_eq!(rows[0].mid, "entry-003");
        assert_eq!(rows[0].ordinal, Some(0));
        assert_eq!(rows[0].written_at, Some(1234));
        let reply = &rows[4].display;
        assert_eq!(reply["parts"][0]["type"], "reasoning");
        assert_eq!(reply["parts"][1]["text"], "Fixture assistant text.");
        assert_eq!(reply["parts"][2]["type"], "tool");
        assert_eq!(reply["parts"][2]["tool"], "head_fixture_image");
        assert_eq!(reply["parts"][2]["callId"], "call_head_fixture");
        assert_eq!(reply["parts"][2]["partKey"], "2");
        assert_eq!(
            reply["parts"][2]["outputBytes"],
            "Fixture tool result.".len()
        );
        assert_eq!(rows[5].display["role"], "tool");

        // Expand cloned recorded payloads, never the checksum-pinned journals.
        for size in [2_047, 2_048, 2_049] {
            let mut call = fixture_entry(&entries, "entry-008");
            let input = "x".repeat(size - 11); // {"text":""} adds eleven JSON bytes.
            call["message"]["message"]["content"][2]["arguments"] = json!({"text":input});
            let rows = source.rows(&[call.clone()], false, &[]).unwrap();
            let part = &rows[0].display["parts"][2];
            assert_eq!(part["arguments"].as_str().unwrap().len(), size.min(2_048));
            assert_eq!(part["argumentsTruncated"], size > 2_048);
            assert_eq!(part["callId"], "call_head_fixture");
            let full = source.rows(&[call], true, &[]).unwrap();
            assert_eq!(
                full[0].display["parts"][2]["arguments"]["text"]
                    .as_str()
                    .unwrap()
                    .len(),
                size - 11
            );
        }
        for is_error in [false, true] {
            let call = fixture_entry(&entries, "entry-008");
            let mut result = fixture_entry(&entries, "entry-009");
            let text = "x".repeat(70_000);
            result["message"]["message"]["content"][0]["text"] = json!(text);
            result["message"]["message"]["isError"] = json!(is_error);
            let pair = [call, result];
            let rows = source.rows(&pair, false, &[]).unwrap();
            let tool = &rows[0].display["parts"][2];
            assert_eq!(tool["outputBytes"], 70_000);
            assert_eq!(tool.get("isError").is_some(), is_error);
            assert!(tool.get("content").is_none(), "a page shows no result text");
            let full = source.rows(&pair, true, &[]).unwrap();
            assert_eq!(full[0].display["parts"][2]["content"], text);
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

/// Broca's stored blocks keep their own decoder; an OpenCode 2 session's go
/// through the contract's display rows, like Pi's.
#[test]
fn each_harness_uses_its_own_row_builder() {
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    let entries = [entry("broca", Some(1), 1234)];
    assert_eq!(source.harness(), "broca");
    assert_eq!(
        source.rows(&entries, false, &[]).unwrap()[0].display,
        broca_entries::rows(&entries, false).unwrap()[0].display
    );

    let mut binding = resolved();
    binding.reference.runner = crate::app_reader::resolve::THALAMUS_RUNNER_MODULE_ID.into();
    binding.harness = "opencode2".into();
    let source = BrocaSource::new(binding, FakeRoute::new());
    let entries = [json!({"mid":"oc2", "ordinal": 1, "message":{
        "type":"assistant", "id":"msg_1", "data":{"content":[{"type":"text", "text":"OpenCode 2 text"}]}
    }})];
    assert_eq!(source.harness(), "opencode2");
    assert_eq!(
        source.rows(&entries, false, &[]).unwrap()[0].display,
        broca_entries::display_rows(
            &entries,
            false,
            broca_entries::DisplayProfile::OpenCode2,
            &[]
        )
        .unwrap()[0]
            .display
    );
}
