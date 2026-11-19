use super::*;
use serde_json::json;

fn text(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn person(parts: Vec<Value>) -> Value {
    json!({"role": "user", "messageId": "m1", "text": "words", "parts": parts})
}

#[test]
fn the_persons_words_and_files_stay_and_host_parts_go() {
    let mut row = person(vec![
        json!({"type": "injected", "host": true, "text": "[Request interrupted by user]"}),
        text("Please check the build."),
        json!({"type": "file", "mime": "image/png"}),
    ]);
    assert!(admit(&mut row, false));
    assert_eq!(row["parts"].as_array().unwrap().len(), 2, "{row}");
    assert_eq!(row["text"], "words", "the person's text stays as served");

    let mut files_only = person(vec![json!({"type": "file", "mime": "image/png"})]);
    assert!(admit(&mut files_only, false));

    let mut host_only = person(vec![json!({"type": "injected", "host": true, "text": "x"})]);
    assert!(!admit(&mut host_only, false));
}

/// Text that is only balanced tagged spans is a host's block, not the
/// person's words; text that mixes them with words stays as served.
#[test]
fn text_made_only_of_tagged_spans_is_left_out() {
    let mut spans = person(vec![text(
        "<system-reminder>\nInbox: 2 notices.\n</system-reminder>\n<context kind=\"x\">a</context>",
    )]);
    assert!(!admit(&mut spans, false));

    let words = "<system-reminder>\nInbox: 1 notice.\n</system-reminder>\nWhat broke?";
    let mut mixed = person(vec![text(words)]);
    assert!(admit(&mut mixed, false));
    assert_eq!(mixed["parts"][0]["text"], words);

    // A block that starts with a reminder is served `injected` whole, the
    // person's words after it included; one with only the reminder goes.
    let injected = |text: &str| json!({"type": "injected", "text": text});
    let mut served = person(vec![
        injected("<system-reminder>\nInbox: 2 notices.\n</system-reminder>"),
        injected(words),
    ]);
    assert!(admit(&mut served, false));
    assert_eq!(served["parts"], json!([injected(words)]));
    let mut reminder = person(vec![injected("<system-reminder>x</system-reminder>")]);
    assert!(!admit(&mut reminder, false));

    // A `<` that opens no balanced span is the person's.
    let mut angle = person(vec![text("<b>bold")]);
    assert!(admit(&mut angle, false));
    let mut less = person(vec![text("a < b")]);
    assert!(admit(&mut less, false));
}

#[test]
fn someone_elses_words_are_left_out() {
    for mark in ["speaksFor", "sentBy", "origin"] {
        let mut row = person(vec![text("Hello")]);
        row[mark] = json!({"module": "wernicke"});
        assert!(!admit(&mut row, false), "{mark}");
    }
    let mut unknown = json!({"role": "unknown", "unreadable": true,
        "parts": [text("[Unreadable transcript entry]")]});
    assert!(!admit(&mut unknown, false));
    let mut unreadable_part = person(vec![json!({"type": "unreadable", "partIndex": 0})]);
    assert!(admit(&mut unreadable_part, false));
}

#[test]
fn an_agent_row_keeps_only_its_say_messages_and_filed_asks() {
    let say = json!({"type": "message", "callId": "c1", "callKey": "k1", "partKey": "2",
        "text": "Build passed.", "level": "chat"});
    let ask = json!({"type": "tool", "tool": "ask", "callId": "c2", "callKey": "k2",
        "input": {"question": "Ship it?"}});
    let mut row = json!({"role": "assistant", "text": "prose",
    "blockSummaries": [{"kind": "tool"}], "files": [{"mime": "image/png"}], "parts": [
        {"type": "reasoning", "text": "thinking"},
        text("prose"),
        {"type": "tool", "tool": "bash", "input": {"command": "ls"}},
        say,
        ask,
        {"type": "tool", "tool": "ask", "input": {"action": "dismiss", "id": "ask_1"}},
    ]});
    assert!(admit(&mut row, false));
    assert_eq!(row["parts"], json!([say, ask]));
    assert_eq!(
        (&row["text"], &row["blockSummaries"], &row["files"]),
        (&json!(""), &json!([]), &json!([])),
        "{row}"
    );

    let mut created = json!({"role": "assistant", "parts": [
        {"type": "tool", "tool": "ask", "input": {"action": "create", "question": "?"}}]});
    assert!(admit(&mut created, false));
    let mut prose = json!({"role": "assistant", "parts": [text("Done."),
        {"type": "tool", "tool": "ask", "input": {"action": "append", "id": "ask_1"}}]});
    assert!(!admit(&mut prose, false));
}

#[test]
fn a_wake_shows_only_when_it_answers_an_ask() {
    let wake = || {
        json!({"role": "user", "notice": {"sender": "prefrontal"}, "text": "<system-reminder>",
            "parts": [text("<system-reminder>Inbox</system-reminder>")]})
    };
    assert!(!admit(&mut wake(), false));
    let mut answered = wake();
    assert!(admit(&mut answered, true));
    assert_eq!(answered["parts"], json!([]));
    assert_eq!(answered["text"], "");
}

#[test]
fn a_simple_version_names_its_rule() {
    let version = version("history:7:incarnation:abc");
    assert_eq!(version, "history:7:incarnation:abc:simple:2");
    assert!(is_version(&version));
    assert!(!is_version("history:7:incarnation:abc"));
}

#[test]
fn an_unpinned_ask_uses_its_actual_action_even_when_arguments_are_cut() {
    for (action, visible) in [("dismiss", false), ("create", true)] {
        let e = json!({"mid":"m","ordinal":1,"message":{"role":"assistant","content":[{"type":"tool_call","tool_call_id":"c","name":"ask","input":{"a":"x".repeat(3000),"action":action}}]}});
        let mut row = crate::broca_entries::rows(&[e], false).unwrap().remove(0);
        assert_eq!(row.display["parts"][0]["argumentsTruncated"], true);
        assert_eq!(admit(&mut row.display, false), visible);
    }
}

#[test]
fn simple_tool_summaries_describe_only_the_retained_calls() {
    let mut row = json!({"role":"assistant","parts":[{"type":"tool","tool":"bash","callId":"x"},{"type":"tool","tool":"ask","input":{"action":"create"}},{"type":"tool","tool":"ask","input":{"action":"dismiss"}}],"toolCalls":[{"name":"bash","callId":"x"},{"name":"ask","arguments":"create"},{"name":"ask","arguments":"dismiss"}]});
    assert!(admit(&mut row, false));
    assert_eq!(
        row["toolCalls"],
        json!([{"name":"ask","arguments":"create"}])
    );
}
