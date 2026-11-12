//! The phone pins the reader sample sets under `test-vectors/` by tag and checksum, so a
//! file that changes after it is published must fail here before anything else reads it.

use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Every file in the set matches its line in the set's `SHA256SUMS`, every file is
/// listed there, and every `.json` file parses. A published set must also never carry
/// the recording machine's user name, so every home path in it is the redacted one.
fn verify_published_set(set: &str, expected_files: usize) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(set);
    let sums = std::fs::read_to_string(root.join("SHA256SUMS")).unwrap();
    let mut listed = Vec::new();
    for line in sums.lines() {
        let (digest, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), digest, "{name}");
        if name.ends_with(".json") {
            let _: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        }
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(
            text.matches("/Users/").count(),
            text.matches("/Users/user-redact/").count(),
            "{name}"
        );
        listed.push(name.to_string());
    }
    let mut present: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "SHA256SUMS" && name != "README.md")
        .collect();
    present.sort();
    listed.sort();
    assert_eq!(listed, present, "every sample file is listed in SHA256SUMS");
    assert_eq!(listed.len(), expected_files);
}

#[test]
fn the_broca_reader_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-broca-v1", 13);
}

/// The part-key set is recorded from the module's handlers over a fake Broca route;
/// `the_part_key_samples_are_what_the_handlers_answer` in `thalamus-harness-module`
/// compares every reply with these files. Here every file must match its checksum.
#[test]
fn the_part_key_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-broca-v2-keys", 8);
}

#[test]
fn the_contract_valid_head_part_key_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-broca-v3-keys", 8);
}

#[test]
fn the_contract_valid_head_session_fact_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-session-facts-v2", 7);
}

/// The sets served once runners moved to llm-runner 0.8. `occipital-module`
/// serves each again and compares the bytes; here every file must match its
/// checksum.
#[test]
fn the_runner_0_8_samples_are_complete_and_their_checksums_verify() {
    for (set, files) in [
        ("phone-reader-broca-v4-keys", 8),
        ("phone-session-facts-v3", 7),
        ("phone-reader-empty-claim-v2", 8),
        ("phone-reader-pi-v2", 6),
        ("phone-reader-tool-times-v3", 4),
        ("phone-reader-tool-display-v4", 22),
        ("phone-reader-files-v1", 2),
        ("phone-uploads-prompt-v1", 24),
        ("phone-reader-files-v2", 10),
        ("phone-uploads-prompt-v2", 30),
        ("phone-reader-broca-renumber-v1", 6),
        ("phone-reader-opencode2-live-v1", 9),
        ("phone-session-tier-v1", 6),
        ("phone-reader-relayed-v1", 4),
        ("phone-reader-history-intake-v1", 6),
        ("phone-reader-host-unknown-v1", 8),
        ("phone-reader-stop-all-v1", 13),
    ] {
        verify_published_set(&format!("test-vectors/{set}"), files);
    }
}

#[test]
fn contract_valid_head_samples_change_only_the_opaque_activity_token() {
    for (old, new, changed_files) in [
        ("phone-session-facts-v1", "phone-session-facts-v2", 2),
        (
            "phone-reader-broca-v2-keys",
            "phone-reader-broca-v3-keys",
            1,
        ),
    ] {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors");
        let old_sums = std::fs::read_to_string(root.join(old).join("SHA256SUMS")).unwrap();
        let mut changed = 0;
        for line in old_sums.lines() {
            let (_, name) = line.split_once("  ").unwrap();
            let before = std::fs::read(root.join(old).join(name)).unwrap();
            let after = std::fs::read(root.join(new).join(name)).unwrap();
            if before == after {
                continue;
            }
            let mut before: serde_json::Value = serde_json::from_slice(&before).unwrap();
            let mut after: serde_json::Value = serde_json::from_slice(&after).unwrap();
            for value in [&mut before, &mut after] {
                let reply = if value.get("result").is_some() {
                    &mut value["result"]
                } else {
                    value
                };
                let token = reply
                    .as_object_mut()
                    .unwrap()
                    .remove("activityToken")
                    .unwrap();
                assert!(token.as_str().unwrap().starts_with("broca-activity-"));
            }
            assert_eq!(before, after, "{old}/{name}: only activityToken may change");
            changed += 1;
        }
        assert_eq!(changed, changed_files, "{old} -> {new}");
    }
}

/// Live text recorded from a real Broca worker through the production module. Its
/// README says what each file shows.
#[test]
fn the_broca_live_text_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-broca-live-v1", 91);
}

/// Sends, session details and message times recorded from real Broca workers
/// through the production module. Its README says what each file shows.
#[test]
fn the_broca_send_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-broca-sends-v1", 20);
}

/// Pages recorded from a Broca worker that ran before Broca recorded write times
/// and from one started after. Its README says what each file shows.
#[test]
fn the_broca_completion_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-broca-completion-v1", 10);
}

/// A send into a running Broca worker and the pages around it, recorded through the
/// production module. Its README says what each file shows.
#[test]
fn the_broca_echo_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-broca-echo-v1", 16);
}

/// The property the echo set was recorded to show: the stored row of the phone's send
/// carries the receipt's `clientMessageId`, and no other row carries one.
#[test]
fn the_recorded_send_is_echoed_on_its_stored_row_only() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/phone-reader-broca-echo-v1");
    let read = |name: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap()
    };
    let receipt = read("06-new-send-receipt.json");
    let sent = receipt["result"]["clientMessageId"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut echoed = Vec::new();
    for name in [
        "02-old-newest-page.json",
        "03-old-older-page.json",
        "04-new-newest-page.json",
        "05-new-older-page.json",
        "07-new-newest-page-after-send.json",
    ] {
        for row in read(name)["result"]["messages"].as_array().unwrap() {
            if let Some(id) = row.get("clientMessageId") {
                assert_eq!(row["role"], "user", "{name}");
                echoed.push((name, row["messageId"].clone(), id.clone()));
            }
        }
    }
    assert_eq!(
        echoed,
        vec![(
            "07-new-newest-page-after-send.json",
            serde_json::json!("m8"),
            serde_json::json!(sent)
        )]
    );
}

/// The property the completion set was recorded to show: every assistant row says
/// when it finished, and no other row carries completion members.
#[test]
fn every_recorded_broca_assistant_row_carries_its_completion_time() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors/phone-reader-broca-completion-v1");
    for name in [
        "02-old-newest-page.json",
        "03-old-older-page.json",
        "04-new-newest-page.json",
        "05-new-older-page.json",
    ] {
        let page: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap();
        let rows = page["result"]["messages"].as_array().unwrap();
        assert!(rows.iter().any(|row| row["role"] == "assistant"), "{name}");
        for row in rows {
            if row["role"] == "assistant" {
                assert_eq!(row["completedAtMs"], row["createdAtMs"], "{name}");
                assert!(row["completedAtMs"].is_i64(), "{name}");
            } else {
                assert!(row.get("completedAtMs").is_none(), "{name}");
            }
            assert!(row.get("completion").is_none(), "{name}");
        }
    }
}

/// A cancelled worker, a gap claim with and without the opt-in, and the newest pages of a
/// long worker, recorded through the production module. Its README says what each shows.
#[test]
fn the_broca_last_run_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-broca-lastrun-v1", 26);
}

/// The properties the last-run set was recorded to show: `lastRun` rides only on replies
/// that carry session facts, the opted-in claim answers `outside_window` where the plain
/// one answers `cursor_expired` with the same resync, and paging back from that resync
/// reaches the turns the phone held.
#[test]
fn the_recorded_gap_claim_resyncs_to_the_newest_page_and_pages_back_to_the_held_turns() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors/phone-reader-broca-lastrun-v1");
    let read = |name: &str| -> serde_json::Value {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap();
        value["result"].clone()
    };
    let ordinals = |page: &serde_json::Value| -> Vec<u64> {
        page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["ordinal"].as_u64().unwrap())
            .collect()
    };

    for name in [
        "01-cancelled-session-details.json",
        "02-cancelled-newest-page.json",
    ] {
        let run = &read(name)["lastRun"];
        assert_eq!(run["state"], "cancelled", "{name}");
        assert!(run.get("error").is_none(), "{name}");
    }
    for name in ["04-gap-older-page.json", "05-gap-held-page.json"] {
        assert!(read(name).get("lastRun").is_none(), "{name}");
    }

    let opted_in = read("06-gap-claim-opted-in.json");
    let plain = read("09-gap-claim-without-accept.json");
    assert_eq!(opted_in["claim"], "outside_window");
    assert_eq!(plain["claim"], "cursor_expired");
    assert_eq!(opted_in["resync"], plain["resync"]);
    assert_eq!(
        ordinals(&opted_in["resync"]),
        (145..=150).collect::<Vec<_>>()
    );
    assert_eq!(opted_in["resync"]["nextBeforeOrdinal"], 145);

    let held = ordinals(&read("05-gap-held-page.json"));
    let back = [read("07-gap-back-1.json"), read("08-gap-back-2.json")];
    assert_eq!(ordinals(&back[0]), (139..=144).collect::<Vec<_>>());
    assert_eq!(ordinals(&back[1]), held);
}

/// The phone reading and sending to an OpenCode session Thalamus's own runner hosts,
/// recorded through the production module. Its README lists what each case shows.
#[test]
fn the_opencode_reader_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-opencode-v1", 52);
}

/// The properties the OpenCode set was recorded to show: a send's `clientMessageId` is
/// echoed onto its user row, claims answer `matched`, the display watch carries the
/// reply's live text, and session details name the send's completed run.
#[test]
fn the_recorded_opencode_reads_echo_the_send_and_stream_its_reply() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/phone-reader-opencode-v1");
    let read = |name: &str| -> serde_json::Value {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap();
        value.get("result").cloned().unwrap_or(value)
    };

    let sent = read("08-send.json");
    let id = sent["clientMessageId"].as_str().unwrap();
    let after = read("09-newest-page-after-send.json");
    let echoed: Vec<_> = after["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["clientMessageId"] == id)
        .collect();
    assert_eq!(echoed.len(), 1, "exactly one row carries the sent id");
    assert_eq!(echoed[0]["role"], "user");
    assert_eq!(echoed[0]["ordinal"], 20);

    for (name, turns) in [("03-claim-current.json", 0), ("04-claim-behind.json", 1)] {
        let claim = read(name);
        assert_eq!(claim["claim"], "matched", "{name}");
        assert_eq!(claim["newTurns"].as_array().unwrap().len(), turns, "{name}");
    }

    let live: Vec<serde_json::Value> = (1..=7)
        .flat_map(|n| {
            let notice = read(&format!("07-display-watch.notice-{n}.json"));
            match notice.get("frames") {
                Some(frames) => frames.as_array().unwrap().clone(),
                None => vec![notice],
            }
        })
        .map(|framed| framed["frame"].clone())
        .filter(|frame| frame["kind"] == "display")
        .collect();
    assert_eq!(live.len(), 4, "one reasoning and three text frames");
    assert!(live
        .iter()
        .all(|frame| frame["messageID"] == live[0]["messageID"]));

    let details = read("11-session-details-after-send.json");
    assert_eq!(details["lastRun"]["state"], "completed");
}

/// The session facts of an OpenCode head Thalamus hosts, recorded through the
/// production module. Its README lists what each case shows.
#[test]
fn the_opencode_session_fact_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-session-facts-opencode-v1", 53);
}

/// Before a counted run the head reports its model and the phone's grant but no spend;
/// after one turn its spend counts that run, starting from it.
#[test]
fn the_recorded_opencode_head_reports_model_grant_and_spend_from_its_first_counted_run() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors/phone-session-facts-opencode-v1");
    let read = |name: &str| -> serde_json::Value {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap();
        value.get("result").cloned().unwrap_or(value)
    };
    let model =
        serde_json::json!({"provider": "anthropic", "model": "claude-opus-5-5", "variant": "high"});
    let before = read("06-session-details.json");
    assert_eq!(before["model"], model);
    assert_eq!(
        before["sender"],
        serde_json::json!({"modes": ["tool_result", "steer", "idle_start"], "interrupt": true})
    );
    assert!(before.get("usage").is_none(), "nothing counted yet");
    assert!(before.get("senderUnknown").is_none());
    assert_eq!(read("01-newest-page.json")["model"], model);
    let after = read("11-session-details-after-send.json");
    let usage = &after["usage"];
    assert_eq!(usage["steps"], 1);
    assert_eq!(usage["since"], after["lastRun"]["runId"]);
    assert_eq!(usage["by_model"][0]["variant"], "high");
    assert_eq!(usage["input_tokens"], usage["by_model"][0]["input_tokens"]);
}

/// OpenCode sessions Thalamus doesn't host, recorded through the production module. Its
/// README lists what each case shows.
#[test]
fn the_no_runner_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-no-runner-v1", 34);
}

/// Reads and sends for a session no runner hosts name that runner as null; an upload
/// into a head no runner hosts still lands in its project.
#[test]
fn the_recorded_no_runner_refusals_name_no_runner_and_the_upload_lands() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/phone-reader-no-runner-v1");
    let read = |name: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap()
    };
    for name in [
        "00-warmup-details.refusal.json",
        "01-unknown-page.refusal.json",
        "02-head-page.refusal.json",
        "08-unknown-upload-begin.refusal.json",
    ] {
        let refusal = read(name);
        assert_eq!(refusal["code"], "session_unknown", "{name}");
        assert_eq!(
            refusal["detail"],
            serde_json::json!({"runner": null}),
            "{name}"
        );
    }
    let send = read("04-head-send.refusal.json");
    assert_eq!(send["code"], "not_found");
    assert_eq!(send["detail"], serde_json::json!({"runner": null}));
    let watch = std::fs::read_to_string(root.join("03-head-display-watch.end.txt")).unwrap();
    assert!(watch.contains("code: \"session_unknown\""), "{watch}");
    assert_eq!(
        read("05-head-upload-begin.request.json")["harness"],
        "opencode"
    );
    assert_eq!(read("06-head-upload-chunk.json")["result"]["committed"], 74);
    let finished = &read("07-head-upload-finish.json")["result"];
    assert_eq!(finished["size"], 74);
    assert!(finished["path"]
        .as_str()
        .unwrap()
        .ends_with("/.cortexkit/uploads/2026-10-28/probe-head-note.txt"));
}

/// A failed tool call per harness, and two calls that aren't failures. Its README lists
/// what each case shows.
#[test]
fn the_tool_error_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-tool-error-v1", 21);
}

/// Each harness marks its failed call with `isError: true` on list and exact reads; a
/// non-zero exit and a missing file reported as output are not failures.
#[test]
fn the_recorded_failed_calls_carry_is_error_and_the_others_do_not() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/phone-reader-tool-error-v1");
    let read = |name: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap()
    };
    let tool = |name: &str, ordinal: u64| -> serde_json::Value {
        let page = read(name);
        let message = page["result"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["ordinal"] == ordinal)
            .unwrap_or_else(|| panic!("{name}: no ordinal {ordinal}"))
            .clone();
        message["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|part| part["type"] == "tool")
            .unwrap_or_else(|| panic!("{name}: no tool part"))
            .clone()
    };
    for name in ["04-head-newest-page.json", "05-head-exact-read.json"] {
        let part = tool(name, 39);
        assert_eq!(part["isError"], true, "{name}");
        assert_eq!(part["status"], "error", "{name}");
        assert!(
            part["error"]
                .as_str()
                .is_some_and(|error| !error.is_empty()),
            "{name}"
        );
    }
    for name in ["10-broca-newest-page.json", "11-broca-exact-read.json"] {
        assert_eq!(tool(name, 6)["isError"], true, "{name}");
    }
    assert_eq!(tool("06-broca-exact-ord2.json", 2)["isError"], false);
    let completed = tool("08-head-exact-ord25.json", 25);
    assert_eq!(completed["status"], "completed");
    assert_ne!(completed["isError"], true);
}

/// A running turn stopped with `session.stop`. Its README lists what each case shows.
#[test]
fn the_stop_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-stop-v1", 28);
}

/// The grant offers Stop, the running run reads `active`, the stop answers `stopped` and
/// its repeat the same, an unknown run is `not_active`, Broca is `unsupported`, and the
/// run reads `cancelled` after, its partial reply kept as a finished row.
#[test]
fn the_recorded_stop_ends_the_named_run_and_keeps_its_reply() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/phone-reader-stop-v1");
    let read = |name: &str| -> serde_json::Value {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap();
        value.get("result").cloned().unwrap_or(value)
    };
    assert_eq!(read("50-stop-details.json")["sender"]["interrupt"], true);
    let running = read("52-stop-details-running.json")["lastRun"].clone();
    assert_eq!(running["state"], "active");
    let run = running["runId"].clone();
    assert_eq!(read("53-stop.request.json")["runId"], run);
    let stopped = read("53-stop.json");
    assert_eq!(
        stopped,
        serde_json::json!({"state": "stopped", "runId": run})
    );
    assert_eq!(
        read("54-stop-repeat.request.json")["clientRequestId"],
        read("53-stop.request.json")["clientRequestId"]
    );
    assert_eq!(read("54-stop-repeat.json"), stopped);
    assert_eq!(read("55-stop-unknown-run.json")["state"], "not_active");
    let broca = read("56-stop-broca.refusal.json");
    assert_eq!(broca["code"], "unsupported");
    assert_eq!(broca["detail"]["field"], "session.stop");
    let page = read("57-stop-newest-page-after.json");
    assert_eq!(
        page["lastRun"],
        serde_json::json!({"runId": run, "state": "cancelled"})
    );
    let partial = page["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["ordinal"] == 45)
        .expect("the partial reply is kept");
    assert_eq!(partial["role"], "assistant");
    assert!(partial["completedAtMs"].is_number());
    assert!(!partial["text"].as_str().unwrap().is_empty());
    assert_eq!(
        read("58-stop-details-after.json")["lastRun"]["state"],
        "cancelled"
    );
}

/// Sends refused because no account can serve them yet, held rather than failed. Its
/// README lists what each case shows; the module's send tests prove each app file.
#[test]
fn the_account_wait_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-account-wait-v1", 6);
}

/// The same, adding a queued send refused at its turn and a turn that ended on the
/// account, with the final code. Its README lists what each case shows; the
/// module's send and session-facts tests prove each app file.
#[test]
fn the_account_wait_v2_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-account-wait-v2", 17);
}

/// A run that ended because its key's account is out of credit
/// (`billing_exhausted`), then a permanent error naming no code. The sample
/// set's README lists what each case shows; the module's session-fact tests
/// prove each file.
#[test]
fn the_out_of_credit_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-out-of-credit-v1", 8);
}

/// A run Broca paused because its account is at its limit: its resume time on
/// details, the newest page and a claim, then the chat once it has finished.
/// The sample set's README lists what each case shows; the module's
/// session-fact tests prove each file.
#[test]
fn the_account_pause_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-account-pause-v1", 8);
}

/// What a session's model is given before the conversation, from the plan in
/// force, and each refusal. Its README lists what each case shows; the module's
/// request-context sample test proves each file.
#[test]
fn the_request_context_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-request-context-v1", 20);
}

/// A head's run held on a tool call, stopped with `session.stop`. Its README lists what
/// each case shows.
#[test]
fn the_held_stop_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-stop-held-v1", 6);
}

/// A run paused on a held call is offered Stop by the grant, the stop names that run and
/// answers `stopped`, and a different run is active after it.
#[test]
fn the_recorded_held_stop_ends_the_paused_run() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/phone-reader-stop-held-v1");
    let read = |name: &str| -> serde_json::Value {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap();
        value.get("result").cloned().unwrap_or(value)
    };
    let before = read("60-held-details.json");
    assert_eq!(before["sender"]["interrupt"], true);
    assert_eq!(before["lastRun"]["state"], "paused");
    assert_eq!(before["lastRun"]["reason"], "tool_held");
    let run = before["lastRun"]["runId"].clone();
    assert_eq!(read("61-held-stop.request.json")["runId"], run);
    assert_eq!(
        read("61-held-stop.json"),
        serde_json::json!({"state": "stopped", "runId": run})
    );
    let after = read("62-held-details-after.json")["lastRun"].clone();
    assert_eq!(after["state"], "active");
    assert_ne!(after["runId"], run);
}

/// Reads around a message found by search, then older and newer pages from it. Its
/// README lists what each case shows.
#[test]
fn the_search_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-search-v1", 29);
}

/// The anchor page holds `limit` messages either side of the anchor; older and newer
/// pages continue from its edges without a gap, the last one reaching the newest message;
/// every page shares one version, which the unknown-message refusal names.
#[test]
fn the_recorded_search_pages_join_without_a_gap() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/phone-reader-search-v1");
    let read = |name: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap()
    };
    let ordinals = |page: &serde_json::Value| -> Vec<u64> {
        page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["ordinal"].as_u64().unwrap())
            .collect()
    };
    let anchor = &read("33-anchor-page.json")["result"];
    let version = anchor["version"].clone();
    assert_eq!(read("33-anchor-page.request.json")["limit"], 3);
    assert_eq!(anchor["anchorOrdinal"], 20);
    assert_eq!(ordinals(anchor), (17..=23).collect::<Vec<_>>());
    assert_eq!(anchor["nextBeforeOrdinal"], 17);
    assert_eq!(anchor["nextAfterOrdinal"], 23);
    assert!(anchor.get("reachedNewest").is_none());

    let older = &read("34-anchor-older-page.json")["result"];
    assert_eq!(
        read("34-anchor-older-page.request.json")["beforeOrdinal"],
        17
    );
    assert_eq!(ordinals(older), [14, 15, 16]);

    let mut after = 23;
    for name in ["35-anchor-newer-page-01", "35-anchor-newer-page-02"] {
        assert_eq!(read(&format!("{name}.request.json"))["afterOrdinal"], after);
        let page = &read(&format!("{name}.json"))["result"];
        let got = ordinals(page);
        assert_eq!(got[0], after + 1, "{name}");
        assert!(got.windows(2).all(|pair| pair[1] == pair[0] + 1), "{name}");
        after = *got.last().unwrap();
        assert_eq!(page["version"], version, "{name}");
    }
    let last = &read("35-anchor-newer-page-02.json")["result"];
    assert_eq!(last["reachedNewest"], true);
    assert!(last.get("nextAfterOrdinal").is_none());
    let newest = &read("31-search-newest-page.json")["result"];
    assert_eq!(
        ordinals(newest).last(),
        Some(&after),
        "the walk ends at the newest message"
    );
    assert_eq!(older["version"], version);

    let missing = read("36-anchor-message-not-found.refusal.json");
    assert_eq!(missing["code"], "message_not_found");
    assert_eq!(missing["detail"], serde_json::json!({"version": version}));
    assert_eq!(
        read("37-anchor-form-mixing.refusal.json")["code"],
        "invalid_request"
    );
}

/// Queued and unsent app sends, recorded through the production module plus one
/// synthetic file. Its README lists what each case shows.
#[test]
fn the_queued_send_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-queued-sends-v1", 55);
}

/// The properties the queued-send recording shows: both sends listed `queued` while
/// waiting and gone once delivered; an opted-in watch gets bare list notices ending in
/// `[]`, a plain watch gets none but its claim answers carry the list; and both stored
/// user rows echo their `clientMessageId`.
#[test]
fn the_recorded_queued_sends_are_listed_until_delivered() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/phone-reader-queued-sends-v1");
    let read = |name: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap()
    };
    let states = |value: &serde_json::Value| -> Vec<String> {
        value["queuedSends"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .map(|entry| entry["state"].as_str().unwrap().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    let first = read("04-send-first.request.json")["clientMessageId"].clone();
    let second = read("05-send-second.request.json")["clientMessageId"].clone();
    for name in [
        "06-newest-page-while-queued.json",
        "07-details-while-queued.json",
    ] {
        let result = &read(name)["result"];
        assert_eq!(states(result), ["queued", "queued"], "{name}");
        assert_eq!(result["queuedSends"][0]["clientMessageId"], first, "{name}");
        assert_eq!(
            result["queuedSends"][1]["clientMessageId"], second,
            "{name}"
        );
    }
    for name in [
        "08-newest-page-after-delivery.json",
        "09-details-after-delivery.json",
    ] {
        assert!(read(name)["result"].get("queuedSends").is_none(), "{name}");
    }
    let echoed: Vec<serde_json::Value> = read("08-newest-page-after-delivery.json")["result"]
        ["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row.get("clientMessageId").cloned())
        .collect();
    assert_eq!(echoed, [first, second]);
    let bare = |prefix: &str, count: usize| -> Vec<serde_json::Value> {
        (1..=count)
            .map(|index| read(&format!("{prefix}.notice-{index}.json")))
            .filter(|notice| {
                notice
                    .as_object()
                    .is_some_and(|o| o.len() == 1 && o.contains_key("queuedSends"))
            })
            .collect()
    };
    let opted = bare("02-transcript-watch-opted-in", 8);
    assert_eq!(
        opted
            .iter()
            .map(|notice| states(notice).len())
            .collect::<Vec<_>>(),
        [1, 2, 1, 0]
    );
    assert_eq!(opted.last().unwrap()["queuedSends"], serde_json::json!([]));
    assert!(bare("03-transcript-watch-plain", 4).is_empty());
    assert_eq!(
        states(&read("03-transcript-watch-plain.notice-2.json")),
        ["queued"]
    );
}

/// A send referring to an uploaded file by path, on a Broca worker and an OpenCode head.
/// Its README lists what each half shows.
#[test]
fn the_upload_reference_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-upload-reference-v1", 32);
}

/// For each half: `upload.begin` names the session's harness, the file lands under the
/// project's `.cortexkit/uploads/`, the send's text carries that path on an
/// `Uploaded file:` line, and the stored user row echoes the send's `clientMessageId`
/// with the agent's reply after it.
#[test]
fn the_recorded_sends_refer_to_their_upload_by_path() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors/phone-reader-upload-reference-v1");
    let read = |name: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap()
    };
    for (prefix, half, harness) in [("0", "broca", "broca"), ("1", "head", "opencode")] {
        let begin = read(&format!("{prefix}0-{half}-upload-begin.request.json"));
        assert_eq!(begin["harness"], harness);
        let path = read(&format!("{prefix}2-{half}-upload-finish.json"))["result"]["path"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            path.ends_with("/.cortexkit/uploads/2026-10-28/probe-image.png"),
            "{path}"
        );
        let send = read(&format!("{prefix}3-{half}-send.request.json"));
        assert!(send.get("parts").is_none());
        let text = send["text"].as_str().unwrap();
        assert!(
            text.ends_with(&format!("\n\nUploaded file: {path}")),
            "{text}"
        );
        let page = read(&format!("{prefix}4-{half}-newest-page-after-send.json"));
        let rows = page["result"]["messages"].as_array().unwrap();
        let index = rows
            .iter()
            .position(|row| row["clientMessageId"] == send["clientMessageId"])
            .unwrap_or_else(|| panic!("{half}: no row echoes the send"));
        assert!(
            rows[index]["text"].as_str().unwrap().contains(&path),
            "{half}"
        );
        assert_eq!(rows[index + 1]["role"], "assistant", "{half}");
        assert!(
            rows[index + 1]["text"]
                .as_str()
                .unwrap()
                .contains("probe-image.png"),
            "{half}"
        );
    }
}

/// The phone's project uploads, recorded through the production module. Its README
/// lists what each case shows.
#[test]
fn the_upload_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-uploads-v1", 84);
}

/// The properties the upload set was recorded to show: a repeated finish gives the same
/// answer, a piece already held is accepted without moving `committed`, a name is cleaned
/// rather than refused, refusals carry their code and detail, and the steer naming the
/// placed path is stored with the receipt's `clientMessageId`.
#[test]
fn the_recorded_uploads_place_clean_names_and_repeat_finishes_unchanged() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/phone-uploads-v1");
    let read = |name: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap()
    };

    assert_eq!(
        read("01-small-begin.json")["result"]["chunk_bytes"],
        1 << 20
    );
    let finished = read("03-small-finish.json");
    assert_eq!(finished, read("04-small-finish-again.json"));
    assert_eq!(
        read("05-small-abort-after-finish.json")["result"]["aborted"],
        false
    );

    assert_eq!(
        read("09-chunked-status-partway.json")["result"]["committed"],
        1 << 20
    );
    assert_eq!(
        read("10-chunked-chunk-0-repeat.json")["result"]["committed"],
        1 << 20
    );
    assert_eq!(read("14-chunked-finish.json")["result"]["size"], 3_145_851);

    assert_eq!(
        read("15-cleaned-name-begin.request.json")["name"],
        "../CON.txt"
    );
    let placed = read("17-cleaned-name-finish.json")["result"]["path"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        placed.ends_with("/.cortexkit/uploads/2026-10-26/_CON.txt"),
        "{placed}"
    );

    for (name, code, detail) in [
        (
            "19-aborted-chunk-offset-gap.json",
            "offset_gap",
            "committed",
        ),
        (
            "20-aborted-finish-incomplete.json",
            "checksum_mismatch",
            "committed",
        ),
        ("23-begin-too-large.json", "upload_too_large", "max_bytes"),
    ] {
        let refusal = read(name);
        assert_eq!(refusal["code"], code, "{name}");
        assert!(refusal["detail"].get(detail).is_some(), "{name}");
        assert!(refusal.get("result").is_none(), "{name}");
    }
    assert_eq!(read("21-aborted-abort.json")["result"]["aborted"], true);
    assert_eq!(
        read("22-aborted-status-after-abort.refusal.json")["code"],
        "upload_unknown"
    );

    let path = finished["result"]["path"].as_str().unwrap().to_string();
    let sent = read("06-send-linking-path.request.json");
    assert!(sent["text"].as_str().unwrap().contains(&path));
    let receipt = &read("06-send-linking-path.json")["result"]["clientMessageId"];
    let page = read("24-newest-page-after-send.json");
    let echoed: Vec<_> = page["result"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row.get("clientMessageId").is_some())
        .collect();
    assert_eq!(echoed.len(), 1);
    assert_eq!(echoed[0]["role"], "user");
    assert_eq!(&echoed[0]["clientMessageId"], receipt);
}

#[path = "fixtures/broca_0_7.rs"]
mod broca_0_7;

#[test]
fn broca_0_7_approved_read_keeps_tool_pairing_and_ignores_dispatch_metadata() {
    for (name, reply) in broca_0_7::samples("approved.session.read.json") {
        let entries = reply["result"]["messages"].as_array().unwrap();
        assert_eq!(
            entries[1]["tool_calls"][0]["call_key"],
            format!("{}:5", reply["result"]["lineage_id"].as_str().unwrap()),
            "{name}"
        );
        for full in [false, true] {
            let rows = occipital_app_reader::broca_entries::rows(entries, full).unwrap();
            assert_eq!(rows.len(), 4, "{name}");
            assert_eq!(rows[1].display["parts"][0]["tool"], "work", "{name}");
            assert_eq!(
                rows[1].display["parts"][0]["callId"], "approved-call",
                "{name}"
            );
            assert!(
                rows[1].display["parts"][0]["outputBytes"].as_u64().unwrap() > 0,
                "{name}"
            );
            if full {
                assert_eq!(rows[1].display["parts"][0]["isError"], false, "{name}");
                assert!(
                    rows[1].display.get("fullContentUnavailable").is_none(),
                    "{name}"
                );
            }
            for row in &rows {
                assert!(row.display.get("unreadable").is_none(), "{name}");
                assert!(!row.display.to_string().contains("call_key"), "{name}");
                assert!(!row.display.to_string().contains("schema_pin"), "{name}");
            }
            let mut legacy = entries.clone();
            legacy[1].as_object_mut().unwrap().remove("tool_calls");
            let old_rows = occipital_app_reader::broca_entries::rows(&legacy, full).unwrap();
            assert_eq!(
                serde_json::to_value(rows.iter().map(|r| &r.display).collect::<Vec<_>>()).unwrap(),
                serde_json::to_value(old_rows.iter().map(|r| &r.display).collect::<Vec<_>>())
                    .unwrap(),
                "{name}: attribution is intentionally not projected"
            );
        }
    }
}

/// Broca's own tool event samples decode into the display lane's tool inputs,
/// with the call's key and tool version when the event names them and without
/// them when it doesn't.
#[test]
fn broca_0_7_tool_events_map_to_tool_starts_and_ends() {
    use occipital_app_reader::display_events::{map, DisplayInput};
    use serde_json::json;
    let identity = |input: Option<DisplayInput>, name: &str| match input {
        Some(DisplayInput::ToolStarted(start)) => {
            assert!(
                !start.call_id.is_empty() && !start.tool.is_empty(),
                "{name}"
            );
            Some((start.call_key, start.schema_pin))
        }
        Some(DisplayInput::ToolFinished(end)) => {
            assert!(!end.call_id.is_empty() && !end.outcome.is_empty(), "{name}");
            None
        }
        other => panic!("{name}: {other:?}"),
    };
    for file in [
        "tool_started.json",
        "tool_finished.json",
        "approved.tool_started.json",
        "approved.tool_finished.json",
        "tool_started.frame.json",
        "tool_finished.frame.json",
        "approved.tool_started.frame.json",
        "approved.tool_finished.frame.json",
    ] {
        for (name, value) in broca_0_7::samples(file) {
            let envelope = if file.ends_with(".frame.json") {
                value
            } else {
                json!({"kind":"control", "event":value})
            };
            if let Some((call_key, schema_pin)) = identity(map(&envelope), &name) {
                assert_eq!(
                    (call_key.as_deref(), schema_pin.as_deref()),
                    (
                        envelope["event"]["call_key"].as_str(),
                        envelope["event"]["schema_pin"].as_str()
                    ),
                    "{name}"
                );
            }
            let mut legacy = envelope;
            for field in ["call_key", "schema_pin"] {
                legacy["event"].as_object_mut().unwrap().remove(field);
            }
            if let Some(identity) = identity(map(&legacy), &name) {
                assert_eq!(identity, (None, None), "{name}: legacy tools have no pin");
            }
        }
    }
}

/// A head's first rich UI reply on Thalamus's OpenCode runner. Its README lists what each
/// file shows.
#[test]
fn the_head_rich_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-head-rich-v1", 28);
}

/// The live text frames and the stored reply carry one and the same rich UI stamp, the
/// user row echoes the send's id, and the user row carries no stamp.
#[test]
fn the_recorded_head_reply_is_stamped_live_and_stored() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/phone-reader-head-rich-v1");
    let read = |name: &str| -> serde_json::Value {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap();
        value.get("result").cloned().unwrap_or(value)
    };
    let page = read("63-newest-page.json");
    let row = |ordinal: u64| {
        page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["ordinal"] == ordinal)
            .cloned()
            .unwrap_or_else(|| panic!("row {ordinal} is on the page"))
    };
    let reply = row(99);
    assert_eq!(reply["role"], "assistant");
    assert!(reply["completedAtMs"].is_number());
    let stamp = reply["richUi"].clone();
    assert_eq!(stamp["dialect"], "1.0");
    assert!(stamp["catalogDigest"]
        .as_str()
        .is_some_and(|d| d.len() == 64));
    let user = row(98);
    assert_eq!(user["role"], "user");
    assert!(user.get("richUi").is_none());
    assert_eq!(
        user["clientMessageId"],
        read("62-send.request.json")["clientMessageId"]
    );
    let exact = read("64-exact-read.json");
    assert_eq!(exact["messages"][0]["ordinal"], 99);
    assert_eq!(exact["messages"][0]["richUi"], stamp);
    let mut live = 0;
    for n in 2..=3 {
        let notice = read(&format!("61-display-watch.notice-{n}.json"));
        for frame in notice["frames"].as_array().unwrap() {
            if frame["frame"]["kind"] == "display" {
                assert_eq!(frame["frame"]["richUi"], stamp, "notice {n}");
                live += 1;
            }
        }
    }
    assert_eq!(live, 2, "one reasoning and one text frame");
    assert_eq!(
        read("61-display-watch.notice-5.json")["frames"][0]["frame"]["kind"],
        "display_end"
    );
}

/// Tool calls' start and answer times on a chat's tool parts. Its README lists
/// what each reply shows; `occipital-module` checks v2's replies are still what
/// it serves. v1, recorded from an OpenCode 1 chat, stays published as it was.
#[test]
fn the_tool_times_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-tool-times-v1", 4);
    verify_published_set("test-vectors/phone-reader-tool-times-v2", 4);
}

/// A chat with no messages, claimed with `heldVersion` alone. Its README lists what
/// each reply shows; `occipital-module` checks the replies are still what it serves.
#[test]
fn the_empty_claim_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-empty-claim-v1", 8);
}

/// Paging back past a moved head's cut. Its README lists what each page shows.
#[test]
fn the_head_history_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-head-history-v1", 3);
}

/// The oldest numbered page starts at the cut and hands over a cursor, not an ordinal.
/// Older pages are unnumbered, oldest first, joined without gaps, and never claim the
/// start while a cursor remains.
#[test]
fn a_moved_head_pages_back_past_its_cut_by_cursor() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/phone-reader-head-history-v1");
    let read = |name: &str| -> serde_json::Value {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap();
        value.get("result").cloned().unwrap_or(value)
    };
    let ids = |page: &serde_json::Value| -> Vec<String> {
        page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["messageId"].as_str().unwrap().to_owned())
            .collect()
    };
    let numbered = read("01-oldest-numbered-page.json");
    let ordinals: Vec<u64> = numbered["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["ordinal"].as_u64().unwrap())
        .collect();
    assert_eq!(ordinals, (5337..5400).collect::<Vec<_>>());
    assert!(numbered.get("nextBeforeOrdinal").is_none());
    assert!(numbered["nextBeforeCursor"].is_string());
    for name in [
        "02-before-cursor-page-1.json",
        "03-before-cursor-page-2.json",
    ] {
        let page = read(name);
        assert_eq!(page["messages"].as_array().unwrap().len(), 50, "{name}");
        assert!(
            page["messages"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row.get("ordinal").is_none()),
            "{name}"
        );
        assert!(page["nextBeforeCursor"].is_string(), "{name}");
        assert!(page.get("reachedStart").is_none(), "{name}");
    }
    let first = ids(&read("02-before-cursor-page-1.json"));
    let second = ids(&read("03-before-cursor-page-2.json"));
    assert!(first.last().unwrap().starts_with("msg_0cd8d2c7e001"));
    // OpenCode message ids sort by creation, so each older page ends before the next one starts.
    assert!(second.last().unwrap() < first.first().unwrap());
    assert!(first.last().unwrap() < &ids(&numbered)[0]);
}

/// A phone message to a worker whose run is cancelled first. Its README lists what
/// each step shows.
#[test]
fn the_steer_run_ended_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-steer-run-ended-v1", 18);
}

/// The message waits as `queued` while its run is active, and once the run ends
/// without taking it, reads `unsent` with reason `run_ended`, on details and the page.
#[test]
fn a_message_whose_run_ends_first_reads_unsent_run_ended() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors/phone-reader-steer-run-ended-v1");
    let read = |name: &str| -> serde_json::Value {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap();
        value.get("result").cloned().unwrap_or(value)
    };
    let sent = read("81-send.request.json")["clientMessageId"].clone();
    assert!(sent.is_string());
    for name in ["82-details-queued.json", "83-newest-page-queued.json"] {
        let reply = read(name);
        assert_eq!(reply["lastRun"]["state"], "active", "{name}");
        let queued = &reply["queuedSends"][0];
        assert_eq!(queued["clientMessageId"], sent, "{name}");
        assert_eq!(queued["state"], "queued", "{name}");
    }
    for name in ["84-details-unsent.json", "85-newest-page-unsent.json"] {
        let reply = read(name);
        assert_eq!(reply["lastRun"]["state"], "interrupted", "{name}");
        let unsent = &reply["queuedSends"][0];
        assert_eq!(unsent["clientMessageId"], sent, "{name}");
        assert_eq!(unsent["state"], "unsent", "{name}");
        assert_eq!(unsent["reason"], "run_ended", "{name}");
        assert!(unsent["endedAtMs"].as_u64().is_some(), "{name}");
    }
}

/// Model names as the apps show them. Its README lists what each file shows.
#[test]
fn the_model_name_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-model-names-v1", 4);
}

/// A Pi chat as the apps read it. Its README lists what each reply shows;
/// `occipital-module` checks the replies are still what it serves.
#[test]
fn the_pi_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-pi-v1", 6);
}

/// What tool providers wrote for a chat's calls, as the apps read it. Its README
/// lists what each file shows. v3 is served from an OpenCode 2 chat, and
/// `occipital-module` checks it is still what it serves. v2, the same set from
/// an OpenCode 1 chat, and v1, an earlier v2 whose templates name an older tool
/// catalog digest, stay published as they were.
#[test]
fn the_tool_display_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-tool-display-v1", 22);
    verify_published_set("test-vectors/phone-reader-tool-display-v2", 22);
    verify_published_set("test-vectors/phone-reader-tool-display-v3", 22);
}

/// Tool calls on the live display lane. Its README lists what each file shows.
#[test]
fn the_tool_frame_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-tool-frames-v1", 8);
}

/// A message streamed under a `live_id` until its host stores it. Its README lists
/// what each file shows.
#[test]
fn the_live_id_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-live-ids-v1", 8);
}

/// One OpenCode head run's live display with two messages. Its README lists what each
/// notice shows.
#[test]
fn the_display_end_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-display-end-v1", 52);
}

/// A stored message's live words end with `display_end`, never a gap: alone for the
/// first message, in one notice with its own words for the second, and once more,
/// alone, when the run finishes.
#[test]
fn each_message_and_the_run_end_with_display_end() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/phone-reader-display-end-v1");
    let notice = |n: usize| -> Vec<(serde_json::Value, String, String)> {
        let value: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join(format!("02-display-watch.notice-{n}.json"))).unwrap(),
        )
        .unwrap();
        let value = value.get("result").cloned().unwrap_or(value);
        // A watch's first notice is the baseline, one `{seq, frame}`; later ones carry
        // `frames`.
        let frames = match value["frames"].as_array() {
            Some(frames) => frames.clone(),
            None => vec![value.clone()],
        };
        frames
            .iter()
            .map(|frame| {
                (
                    frame["seq"].clone(),
                    frame["frame"]["kind"].as_str().unwrap().to_owned(),
                    frame["frame"]["messageID"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                )
            })
            .collect()
    };
    let kinds = |n: usize| {
        notice(n)
            .into_iter()
            .map(|(_, kind, _)| kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds(1), ["display_gap"]);
    for n in 2..=5 {
        assert_eq!(kinds(n), ["display"], "notice {n}");
    }
    assert_eq!(kinds(6), ["display_end"]);
    assert_eq!(kinds(7), ["display", "display_end"]);
    assert_eq!(kinds(8), ["display_end"]);
    let first = &notice(2)[0].2;
    let second = &notice(7)[0].2;
    assert!(!first.is_empty() && !second.is_empty());
    assert_ne!(first, second, "the closing sentence is its own message");
    for n in 3..=5 {
        assert_eq!(&notice(n)[0].2, first, "notice {n}");
    }
    let seqs: Vec<_> = (2..=8)
        .flat_map(notice)
        .map(|(seq, _, _)| seq.as_u64().unwrap())
        .collect();
    assert_eq!(
        seqs,
        (2..=9).collect::<Vec<_>>(),
        "frames arrive in order, none lost"
    );
}

/// A refused send's `detail` says whether to resend; the recorded resend under the same
/// `clientMessageId` landed once. Its README lists each file.
#[test]
fn the_runner_refused_samples_are_complete_and_their_checksums_verify() {
    verify_published_set("test-vectors/phone-reader-runner-refused-v1", 21);
}

#[test]
fn a_resend_under_the_same_id_lands_once_and_refusals_carry_their_class() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors/phone-reader-runner-refused-v1");
    let read = |name: &str| -> serde_json::Value {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap();
        value.get("result").cloned().unwrap_or(value)
    };
    let (first, again) = (read("01-send.json"), read("03-send-again-same-id.json"));
    assert_eq!(first["duplicate"], false);
    assert_eq!(again["duplicate"], true);
    assert_eq!(first["effectId"], again["effectId"]);
    assert_eq!(first["clientMessageId"], again["clientMessageId"]);
    for (name, class) in [
        ("06-runner-refused-transient.synthetic.json", "transient"),
        (
            "07-runner-refused-transient-wait.synthetic.json",
            "transient",
        ),
        ("08-runner-refused-final.synthetic.json", "final"),
    ] {
        let refusal = read(name);
        assert_eq!(refusal["code"], "runner_refused", "{name}");
        assert_eq!(refusal["detail"]["class"], class, "{name}");
        assert!(refusal["detail"]["runnerCode"].is_string(), "{name}");
    }
    assert_eq!(
        read("07-runner-refused-transient-wait.synthetic.json")["detail"]["retryAfterMs"],
        1500
    );
}
