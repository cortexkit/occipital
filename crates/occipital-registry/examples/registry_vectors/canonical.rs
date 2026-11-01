use super::{Kind, Result, Vectors};

pub fn add(v: &mut Vectors) -> Result<()> {
    for codepoint in 0..32 {
        v.accepted(
            &format!("canon-control-{codepoint:02x}"),
            format!(r#""\u{codepoint:04X}""#),
            Kind::Json,
        )?;
    }
    for (name, input) in [
        ("short-backspace", r#""\b""#),
        ("short-tab", r#""\t""#),
        ("short-newline", r#""\n""#),
        ("short-formfeed", r#""\f""#),
        ("short-return", r#""\r""#),
        ("del-escaped", r#""\u007f""#),
        ("del-raw", "\"\u{7f}\""),
        ("slash", r#""/""#),
        ("line-separator", r#""\u2028""#),
        ("paragraph-separator", r#""\u2029""#),
        ("separators-raw", "\"\u{2028}\u{2029}\""),
        ("accent-escaped", r#""\u00e9""#),
        ("non-ascii", "\"café 日本\""),
        ("surrogate-pair", r#""\ud83d\ude00""#),
        ("u0080", r#""\u0080""#),
        ("quote", r#""\"""#),
        ("backslash", r#""\\""#),
        ("quote-unicode", r#""\u0022""#),
        ("backslash-unicode", r#""\u005C""#),
        ("letter-escape", r#""\u0061""#),
        ("slash-escape", r#""\/""#),
        ("ascii-escape", r#""\u0041\u0020\u007E""#),
        ("whitespace", " \t\r\n[ 1 ,\t2 ,\r\n3 ] \n"),
        ("space-in-string", "{ \"a b\" : \" x \" }"),
        ("literals", "[ true , false , null , { } , [ ] ]"),
        (
            "nested",
            r#"{"b":[3,1,2],"a":{"z":null,"y":[{"d":1,"c":2}]}}"#,
        ),
        ("array-order", r#"[{"b":1,"a":2},{"a":3}]"#),
        ("non-bmp-low", r#"{"\ud83d\ude01":1,"\ud83d\ude00":2}"#),
        ("non-bmp-high", r#"{"\ud83e\udd00":1,"\ud83d\udfff":2}"#),
        ("key-prefix", r#"{"ab":1,"a":2,"B":3,"b":4}"#),
        ("utf16-order", r#"{"\ue000":1,"\ud800\udc00":2}"#),
        (
            "rfc8785-sort",
            r#"{"\u20ac":"Euro Sign","\r":"Carriage Return","\ufb33":"Hebrew Letter Dalet With Dagesh","1":"One","\ud83d\ude00":"Emoji: Grinning Face","\u0080":"Control","\u00f6":"Latin Small Letter O With Diaeresis"}"#,
        ),
        ("integer-min", "-9007199254740991"),
        ("integer-max", "9007199254740991"),
    ] {
        v.accepted(&format!("canon-{name}"), input, Kind::Json)?;
    }
    v.accepted("canon-depth-64", nested(64), Kind::Json)?;
    // A string uses the full byte budget without depending on whitespace removal.
    v.accepted(
        "canon-input-at-cap",
        format!("\"{}\"", "a".repeat(1_048_574)),
        Kind::Json,
    )?;
    for (name, input, code) in [
        ("fraction", "1.0", "number_not_integer"),
        ("exponent", "1e2", "number_not_integer"),
        ("exponent-upper", "1E0", "number_not_integer"),
        ("exponent-negative", "100E-2", "number_not_integer"),
        ("negative-zero", "-0", "number_not_integer"),
        ("integer-over", "9007199254740992", "number_out_of_range"),
        ("integer-under", "-9007199254740992", "number_out_of_range"),
        (
            "duplicate-escaped-key",
            r#"{"a":1,"\u0061":2}"#,
            "duplicate_key",
        ),
        ("high-surrogate", r#""\ud800""#, "lone_surrogate"),
        ("low-surrogate", r#""\udc00""#, "lone_surrogate"),
        ("nan", "NaN", "json_syntax"),
        ("leading-zero", "01", "json_syntax"),
        ("leading-plus", "+1", "json_syntax"),
        ("trailing-comma", "[1,]", "json_syntax"),
        ("trailing-bytes", "true false", "json_syntax"),
    ] {
        v.refused(&format!("canon-{name}"), input, Kind::Json, code)?;
    }
    v.refused(
        "canon-invalid-utf8",
        b"\"\xff\"",
        Kind::Json,
        "invalid_utf8",
    )?;
    v.refused("canon-depth-65", nested(65), Kind::Json, "depth_exceeded")?;
    v.refused(
        "canon-input-over-cap",
        format!("\"{}\"", "a".repeat(1_048_575)),
        Kind::Json,
        "input_too_large",
    )?;
    for (name, input, code) in [
        (
            "duplicate-before-number",
            br#"{"a":1,"a":1.0}"#.as_slice(),
            "duplicate_key",
        ),
        (
            "number-before-duplicate",
            br#"{"a":1.0,"a":2}"#,
            "number_not_integer",
        ),
        ("invalid-outside-string", b"[\xff]", "invalid_utf8"),
        ("invalid-after-number", b"[1.0,\xff]", "number_not_integer"),
        ("invalid-before-number", b"[\xff,1.0]", "invalid_utf8"),
        ("syntax-before-invalid", b"[x,\xff]", "json_syntax"),
        (
            "surrogate-before-invalid",
            b"[\"\\ud800\",\"\xff\"]",
            "lone_surrogate",
        ),
        (
            "invalid-before-surrogate",
            b"[\"\xff\\ud800\"]",
            "invalid_utf8",
        ),
        ("invalid-at-syntax-offset", b"1.\xff", "invalid_utf8"),
        ("incomplete-number", b"1.", "json_syntax"),
        ("negative-leading-zero", b"-01", "json_syntax"),
    ] {
        v.refused(&format!("precedence-canon-{name}"), input, Kind::Json, code)?;
    }
    let mut oversize = b"NaN".to_vec();
    oversize.resize(1_048_577, b' ');
    v.refused(
        "precedence-canon-size-before-syntax",
        oversize,
        Kind::Json,
        "input_too_large",
    )?;
    Ok(())
}

fn nested(depth: usize) -> String {
    format!("{}0{}", "[".repeat(depth), "]".repeat(depth))
}
