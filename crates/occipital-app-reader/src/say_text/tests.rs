use super::*;

/// Feed `chunks` in order and give back each chunk's text.
fn read(chunks: &[&str]) -> Vec<String> {
    let mut text = SayText::new();
    chunks.iter().map(|chunk| text.feed(chunk)).collect()
}

/// Feed `json` one character at a time, the finest split a runner can make.
fn by_char(json: &str) -> String {
    let mut text = SayText::new();
    json.chars()
        .map(|c| text.feed(c.encode_utf8(&mut [0; 4])))
        .collect()
}

#[test]
fn the_text_grows_chunk_by_chunk() {
    assert_eq!(
        read(&[
            r#"{"text": "Build pa"#,
            r#"ssed."#,
            r#"", "level": "chat"}"#
        ]),
        ["Build pa", "ssed.", ""]
    );
}

/// Members before and after `text`, nested values included, are skipped,
/// so the order the model writes them in doesn't matter.
#[test]
fn other_members_are_skipped_in_any_order() {
    let json = r#"{"reply_to": "m118", "quote": "Did it \"pass\"?", "media": [{"kind": "link", "url": "https://x/{a}"}, 3, null],
        "n": -1.5e3, "ok": true, "text": "Yes, it passed.", "key": "build", "done": false}"#;
    assert_eq!(by_char(json), "Yes, it passed.");
    assert_eq!(read(&[json]).concat(), "Yes, it passed.");
}

/// Escapes decode wherever the chunks split them, including between the two
/// halves of a surrogate pair.
#[test]
fn escapes_split_across_chunks_decode() {
    let json = r#"{"text": "a\"b\\c\/d\n\t\u00e9 \ud83d\ude80 end"}"#;
    let expected = "a\"b\\c/d\n\té 🚀 end";
    assert_eq!(by_char(json), expected);
    for at in 0..json.len() {
        if !json.is_char_boundary(at) {
            continue;
        }
        let (first, second) = json.split_at(at);
        assert_eq!(read(&[first, second]).concat(), expected, "split at {at}");
    }
}

#[test]
fn a_lone_surrogate_reads_as_a_replacement_character() {
    assert_eq!(by_char(r#"{"text": "x\ud83dy"}"#), "x\u{FFFD}y");
    assert_eq!(by_char(r#"{"text": "x\ude80y"}"#), "x\u{FFFD}y");
}

/// Only the top-level `text` counts: a nested one, a repeated one, or a key
/// that merely starts with `text` gives back nothing.
#[test]
fn only_the_first_top_level_text_counts() {
    assert_eq!(
        by_char(r#"{"media": [{"text": "no"}], "texts": "no", "text": "yes", "text": "again"}"#),
        "yes"
    );
    assert_eq!(by_char(r#"{"text": 42, "quote": "no"}"#), "");
}

/// Input that isn't an object, or breaks off malformed, stops the reader
/// for good, keeping what it already gave back.
#[test]
fn malformed_input_stops_the_reader() {
    assert_eq!(by_char(r#"["text", "no"]"#), "");
    assert_eq!(
        read(&[r#"{"text": "fine \q"#, r#" more", "x": 1}"#]),
        ["fine ", ""]
    );
    let mut text = SayText::new();
    assert_eq!(text.feed(r#"{"a": 1 "text": "no"}"#), "");
    assert_eq!(text.feed(r#"{"text": "still no"}"#), "");
}

/// Every character after the object closes is ignored.
#[test]
fn nothing_is_read_after_the_object_closes() {
    assert_eq!(
        read(&[r#"{"text": "done"}"#, r#"{"text": "more"}"#]),
        ["done", ""]
    );
}
