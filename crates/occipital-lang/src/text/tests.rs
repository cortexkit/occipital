use super::DisplayText;
use crate::{ScalarString, Value, ValueKind};

#[test]
fn tree_strings_replace_every_c0_c1_control_except_tab_and_lf() {
    for scalar in (0..=0x1f).chain(0x7f..=0x9f) {
        let c = char::from_u32(scalar).unwrap();
        let input = format!("before{c}after");
        let expected = if scalar == 9 || scalar == 10 {
            input.clone()
        } else {
            "before\u{FFFD}after".into()
        };
        assert_eq!(
            DisplayText::new(&input).as_str(),
            expected,
            "U+{scalar:04X}"
        );
    }
}

#[test]
fn tree_strings_preserve_all_other_scalars_and_raw_values() {
    let raw = "\r\u{1B}[31m\u{85} e\u{301} é ß İ \u{200B} \u{202E} 🦀";
    let value = Value::string(raw).unwrap();
    let ValueKind::String(raw_value) = value.kind() else {
        panic!("expected a string");
    };
    assert_eq!(raw_value.as_str(), raw);
    assert_eq!(ScalarString::new(raw).as_str(), raw);
    let display = DisplayText::new(raw_value.as_str());
    assert_eq!(
        display.as_str(),
        "�\u{FFFD}[31m� e\u{301} é ß İ \u{200B} \u{202E} 🦀"
    );
    assert_eq!(DisplayText::new(display.as_str()), display);
    assert_eq!(raw_value.as_str(), raw);
}

#[test]
fn fallback_repairs_unparsed_bytes_and_neutralizes_terminal_controls() {
    assert_eq!(
        DisplayText::from_bytes(b"Title\r\n\x1b]52;c;secret\x07\xff\xc3\xa9\tend").as_str(),
        "Title�\n�]52;c;secret��é\tend"
    );
    assert_eq!(DisplayText::from_bytes(b""), DisplayText::new(""));
}

#[test]
fn failed_tag_literals_are_sanitized_without_reparsing_or_escaping_markup() {
    assert_eq!(
        DisplayText::from_bytes(b"<Gauge label=\"\x1b[0m\" />\n**{total}** &amp;\x00").as_str(),
        "<Gauge label=\"�[0m\" />\n**{total}** &amp;�"
    );
}

#[test]
fn address_sheets_escape_controls_and_pinned_invisible_properties() {
    assert_eq!(
        DisplayText::address_sheet("https://xn--bcher-kva.example/a\u{200B}?x=\u{202E}y\u{FE0F}\u{E0100}\u{61C}\u{200E}\u{AD}").as_str(),
        "https://xn--bcher-kva.example/a\\u{200B}?x=\\u{202E}y\\u{FE0F}\\u{E0100}\\u{61C}\\u{200E}\\u{AD}"
    );
    assert_eq!(
        DisplayText::address_sheet("\0\t\n\r\u{1B}\u{7F}\u{85}\u{9F}").as_str(),
        "\\u{0}\\u{9}\\u{A}\\u{D}\\u{1B}\\u{7F}\\u{85}\\u{9F}"
    );
    // U+2065 and U+E0000 are reserved default ignorables in the pinned table.
    assert_eq!(
        DisplayText::address_sheet("\u{2065}\u{E0000}").as_str(),
        "\\u{2065}\\u{E0000}"
    );
}

#[test]
fn address_sheets_keep_the_complete_suffix_and_do_not_normalize_or_escape_other_text() {
    let input = format!(
        "https://example.test/{}?last=é+e\u{301}&literal=\\u{{202E}}",
        "x".repeat(2048)
    );
    assert_eq!(DisplayText::address_sheet(&input).as_str(), input);
    assert_eq!(
        DisplayText::address_sheet("🦀 <>&\" \u{180E}").as_str(),
        "🦀 <>&\" \\u{180E}"
    );
}

#[test]
fn serialized_display_text_and_plain_text_share_the_sanitizer() {
    let text = DisplayText::new("\u{1B}[31mred\r\n\t\u{85}");
    assert_eq!(text.to_string(), "�[31mred�\n\t�");
    assert_eq!(
        serde_json::to_value(text).unwrap(),
        serde_json::json!("�[31mred�\n\t�")
    );
}
