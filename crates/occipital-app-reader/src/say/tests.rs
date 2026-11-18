use super::*;

/// A row with one `say` call as row building marks it, `outcome` merged into
/// its tool part, beside a plain `read` call.
fn row(outcome: Value) -> Value {
    let mut say = json!({"type": "tool", "tool": "say", "arguments": "{\"text\":\"Bu…",
        "argumentsTruncated": true, "partIndex": 1, "partKey": "1", "callId": "c1", "callKey": "ck_1"});
    mark(
        &mut say,
        "say",
        &json!({"text": "Build passed. <Badge tone=\"success\">ok</Badge>", "level": "attention",
            "key": "build", "done": true, "reply_to": "479640"}),
    );
    for (name, value) in outcome.as_object().unwrap() {
        say[name] = value.clone();
    }
    json!({"role": "assistant", "text": "Checking.", "blockSummaries": ["read", "say"],
        "toolCalls": [{"name": "read", "callId": "c0"}, {"name": "say", "callId": "c1"}],
        "parts": [{"type": "text", "text": "Checking.", "partIndex": 0, "partKey": "0"},
            say,
            {"type": "tool", "tool": "read", "partIndex": 2, "partKey": "2", "callId": "c0", "outputBytes": 3}]})
}

/// An answered `say` on a chat whose agent has prefrontal's `say` becomes the
/// message, with its whole text and settings, and leaves the row's tool calls.
#[test]
fn an_answered_say_becomes_the_message() {
    let mut display = row(json!({"outputBytes": 4}));
    resolve(&mut display, true);
    assert_eq!(
        display["parts"][1],
        json!({"type": "message", "partIndex": 1, "partKey": "1", "callId": "c1", "callKey": "ck_1",
            "text": "Build passed. <Badge tone=\"success\">ok</Badge>",
            "level": "attention", "key": "build", "done": true})
    );
    assert_eq!(
        display["toolCalls"],
        json!([{"name": "read", "callId": "c0"}])
    );
    assert_eq!(display["blockSummaries"], json!(["read"]));
    assert_eq!(display["parts"][2]["type"], "tool");
}

/// The same call stays a plain tool card where the agent's `say` isn't
/// prefrontal's, and the mark never reaches the apps.
#[test]
fn elsewhere_a_say_stays_a_tool_card() {
    let mut display = row(json!({"outputBytes": 4}));
    resolve(&mut display, false);
    assert_eq!(display["parts"][1]["type"], "tool");
    assert!(!any_marked([&display]));
    assert!(display["parts"][1].get("saySettings").is_none());
    assert_eq!(display["toolCalls"].as_array().unwrap().len(), 2);
}

/// A failed or unanswered `say` sent no message: it stays a tool card.
#[test]
fn a_failed_say_stays_a_tool_card() {
    for outcome in [
        json!({"outputBytes": 40, "isError": true}),
        json!({"unanswered": "interrupted"}),
    ] {
        let mut display = row(outcome.clone());
        resolve(&mut display, true);
        assert_eq!(display["parts"][1]["type"], "tool", "{outcome}");
        assert!(!any_marked([&display]), "{outcome}");
    }
}

/// A `say` still running is left out of the row until it's answered.
#[test]
fn a_running_say_is_left_out() {
    let mut display = row(json!({}));
    resolve(&mut display, true);
    let kinds: Vec<&Value> = display["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| &part["type"])
        .collect();
    assert_eq!(kinds, [&json!("text"), &json!("tool")]);
    assert_eq!(
        display["toolCalls"],
        json!([{"name": "read", "callId": "c0"}])
    );
}

/// An exact read's answered call (with `content`) becomes the message too.
#[test]
fn an_exact_read_s_answered_say_becomes_the_message() {
    let mut display = row(json!({"content": "sent", "isError": false}));
    resolve(&mut display, true);
    assert_eq!(display["parts"][1]["type"], "message");
}

/// Settings that aren't what `say` takes are left out, and `done` counts
/// only with a key.
#[test]
fn bad_settings_are_left_out() {
    let mut part = json!({"type": "tool", "tool": "say", "outputBytes": 1, "callId": "c"});
    mark(
        &mut part,
        "say",
        &json!({"text": "hi", "level": "loud", "key": "k".repeat(65), "done": true}),
    );
    let mut display = json!({"parts": [part]});
    resolve(&mut display, true);
    assert_eq!(
        display["parts"][0],
        json!({"type": "message", "callId": "c", "text": "hi"})
    );
}

/// Only a call named `say` with a string `text` is marked.
#[test]
fn only_a_say_with_text_is_marked() {
    for (name, input) in [
        ("say", json!({"text": 3})),
        ("say", json!("not an object")),
        ("write", json!({"text": "x"})),
    ] {
        let mut part = json!({"type": "tool"});
        mark(&mut part, name, &input);
        assert!(!any_marked([&json!({"parts": [part]})]), "{name} {input}");
    }
}
