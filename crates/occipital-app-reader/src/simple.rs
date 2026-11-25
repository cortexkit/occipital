//! The apps' Simple view of a chat: the person's messages, what the agent says
//! through `say`, the asks it files, and the person's answers to asks, without
//! the tool calls, reasoning and prose between them. A read asks for it with
//! `view: "simple"`, and Occipital filters the chat for it, so the apps read
//! only the messages the view shows.
//!
//! The rule is versioned ([`RULE`]): its number is part of every Simple page's
//! version, so a changed rule makes the apps read their Simple copies again
//! while their full copies stay. Ordinals and message ids are the full
//! history's, so a message is the same message in both copies.
//!
//! [`admit`] decides a row and trims it to what the view draws; [`tails`]
//! finds a chat's Simple rows and keeps them between reads.

pub mod tails;

use serde_json::Value;

use crate::broca_entries::PART_KEY;

/// The value of a read's `view` member that asks for this view.
pub const VIEW: &str = "simple";
/// The rule's version. Raise it whenever [`admit`] changes what it admits or
/// keeps, so the apps drop the Simple copies an earlier rule built.
pub const RULE: u32 = 2;

/// Internal override when projecting a call's arguments loses its actual action.
/// The full and Simple wire rows never carry this member.
const FILES_ASK: &str = "_filesAsk";

fn files_ask(input: &Value) -> bool {
    match input.get("action") {
        None | Some(Value::Null) => true,
        Some(action) => action == "create",
    }
}

pub(crate) fn note_ask(part: &mut Value, name: &str, arguments: &Value) {
    if name == "ask" && files_ask(arguments) != files_ask(&part["input"]) {
        part[FILES_ASK] = Value::Bool(files_ask(arguments));
    }
}

pub(crate) fn clear_ask_marks(display: &mut Value) {
    if let Some(parts) = display.get_mut("parts").and_then(Value::as_array_mut) {
        for part in parts.iter_mut().filter_map(Value::as_object_mut) {
            part.remove(FILES_ASK);
        }
    }
}

/// The Simple version of a page whose full version is `full`.
pub fn version(full: &str) -> String {
    format!("{full}:{VIEW}:{RULE}")
}

/// Whether `version` names a Simple page.
pub fn is_version(version: &str) -> bool {
    version.ends_with(&format!(":{VIEW}:{RULE}"))
}

/// Where the reader keeps the Simple copies it served of `session`, apart
/// from the full ones. A claim is checked against the copy its claimant was
/// served, and a chat keeps a bounded number of copies, so in one shared
/// list a busy chat's full reads would push out a Simple claimant's copy and
/// a row that changed since would go unnoticed.
pub(crate) fn claims_key(session: &str) -> String {
    format!("{session}\u{1f}{VIEW}")
}

/// Whether the Simple view shows the full row `display`, trimming it to what
/// the view draws when it does. `answers_ask` says the row is a wake whose
/// record holds the person's answer to an ask, which only the module that
/// keeps wake records knows.
///
/// - The person's message: role `user`, none of the marks that make it
///   someone else's words, and still some text, a file or an unreadable part
///   once host-written parts are dropped. Its other parts stay as served.
/// - The agent's `say` messages and the asks it files: an assistant row keeps
///   only its `message` parts and its `ask` calls that file an ask, each
///   whole, and none of its prose, reasoning or other calls.
/// - The person's answer to an ask: a wake row, with no parts, since the apps
///   draw it from the wake record.
pub fn admit(display: &mut Value, answers_ask: bool) -> bool {
    let role = display["role"].as_str().unwrap_or_default().to_owned();
    if display.get("notice").is_some() {
        if !answers_ask {
            return false;
        }
        keep_parts(display, |_| false);
        return true;
    }
    match role.as_str() {
        "user" => {
            const OTHERS: [&str; 3] = ["speaksFor", "sentBy", "origin"];
            if OTHERS.iter().any(|mark| display.get(*mark).is_some()) {
                return false;
            }
            let parts = display["parts"].as_array().cloned().unwrap_or_default();
            if !parts.iter().any(persons_own) {
                return false;
            }
            if let Some(parts) = display["parts"].as_array_mut() {
                parts.retain(|part| part["type"] != "injected" || persons_own(part));
            }
            trim_host_frames(display);
            true
        }
        "assistant" => {
            let parts = display["parts"].as_array().cloned().unwrap_or_default();
            if !parts.iter().any(drawn_for_agent) {
                return false;
            }
            keep_parts(display, drawn_for_agent);
            true
        }
        _ => false,
    }
}

/// Keep only the parts `keep` admits, and empty the row's summaries of the
/// parts it no longer holds: its text, its block summaries and its files.
/// They stay present, since every app row carries them.
fn keep_parts(display: &mut Value, keep: impl Fn(&Value) -> bool) {
    // The call summaries and named tool parts are built in the same order.
    // Match by position so older calls without a callId remain distinct.
    let mut keep_calls = Vec::new();
    if let Some(parts) = display["parts"].as_array_mut() {
        parts.retain(|part| {
            let retained = keep(part);
            if part["type"] == "tool" && part["tool"].is_string() {
                keep_calls.push(retained);
            }
            retained
        });
    }
    if let Some(calls) = display.get_mut("toolCalls").and_then(Value::as_array_mut) {
        let mut keep = keep_calls.into_iter();
        calls.retain(|_| keep.next().unwrap_or(false));
        if calls.is_empty() {
            if let Some(row) = display.as_object_mut() {
                row.remove("toolCalls");
            }
        }
    }
    if let Some(row) = display.as_object_mut() {
        row.insert("text".into(), Value::String(String::new()));
        row.insert("blockSummaries".into(), Value::Array(Vec::new()));
        if row.contains_key("files") {
            row.insert("files".into(), Value::Array(Vec::new()));
        }
    }
    trim_host_frames(display);
}

/// Keep only the `hostFrames` entries whose words part the row still holds,
/// so each names a part the apps can draw, and drop the member once none
/// does. A wrapped message whose words are all a host's tagged notice is one
/// the view leaves out.
fn trim_host_frames(display: &mut Value) {
    let held: Vec<String> = display["parts"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part[PART_KEY].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let Some(row) = display.as_object_mut() else {
        return;
    };
    let Some(frames) = row.get_mut("hostFrames").and_then(Value::as_array_mut) else {
        return;
    };
    frames.retain(|frame| {
        frame["partKey"]
            .as_str()
            .is_some_and(|key| held.iter().any(|held| held == key))
    });
    if frames.is_empty() {
        row.remove("hostFrames");
    }
}

/// A part of a person's row the view draws: text that is still there once
/// tagged spans a host put in are taken out, a file, or an unreadable part,
/// which the apps draw as a small "couldn't be read" marker. A text block that starts with such a span is
/// served `injected` whole, so the person's words after it count too; a part
/// the host wrote (`host`) never does.
fn persons_own(part: &Value) -> bool {
    let words = || {
        part["text"]
            .as_str()
            .is_some_and(|text| !without_tagged_spans(text).trim().is_empty())
    };
    match part["type"].as_str() {
        Some("text") => words(),
        Some("injected") => part["host"] != true && words(),
        Some("file" | "unreadable") => true,
        _ => false,
    }
}

/// A part of an agent's row the view draws: a `say` message, or an `ask`
/// call that files an ask (its input names no other action).
fn drawn_for_agent(part: &Value) -> bool {
    match part["type"].as_str() {
        Some("message") => true,
        Some("tool") => {
            part["tool"] == "ask"
                && part[FILES_ASK]
                    .as_bool()
                    .unwrap_or_else(|| files_ask(&part["input"]))
        }
        _ => false,
    }
}

/// `text` with each balanced `<name …>…</name>` span taken out, as the apps
/// strip blocks a host put into the person's text (a `<system-reminder>`, for
/// one). A `<` that opens no such span stays.
fn without_tagged_spans(text: &str) -> String {
    let mut kept = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        let span = tagged_span(&rest[open..]);
        match span {
            Some(length) => {
                kept.push_str(&rest[..open]);
                rest = &rest[open + length..];
            }
            None => {
                kept.push_str(&rest[..=open]);
                rest = &rest[open + 1..];
            }
        }
    }
    kept.push_str(rest);
    kept
}

/// The length of the `<name …>…</name>` span `text` starts with, if it starts
/// with one: a name of letters, digits, `-` and `_` that starts with a
/// letter, then `>` or a space, and the first matching closing tag.
fn tagged_span(text: &str) -> Option<usize> {
    let after = text.strip_prefix('<')?;
    let name_length = after
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        .count();
    let name = &after[..name_length];
    if !name.bytes().next()?.is_ascii_alphabetic() {
        return None;
    }
    let tag = &after[name_length..];
    if !(tag.starts_with('>') || tag.starts_with(char::is_whitespace)) {
        return None;
    }
    let body = 1 + name_length + tag.find('>')? + 1;
    let close = format!("</{name}>");
    let end = text[body..].find(&close)?;
    Some(body + end + close.len())
}

#[cfg(test)]
mod tests;
