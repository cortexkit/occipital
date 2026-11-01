//! Canonical JSON tests: one refusal vector per error code with its offset,
//! the boundary pairs, the offset rules between faults, and a hand-written
//! table of inputs and their expected canonical bytes. Every expected value
//! here is written out by hand, never produced by the crate.

use occipital_registry::canon::{
    canonicalize, parse, CanonErrorKind, Integer, Object, Value, MAX_DEPTH, MAX_INPUT_BYTES,
};

/// Asserts that `input` is refused with `kind` at `offset`.
#[track_caller]
fn refused(input: &[u8], kind: CanonErrorKind, offset: usize) {
    match canonicalize(input) {
        Ok(bytes) => panic!(
            "{:?} was accepted as {:?}, expected {} at {offset}",
            String::from_utf8_lossy(input),
            String::from_utf8_lossy(&bytes),
            kind.code()
        ),
        Err(error) => assert_eq!(
            (error.kind(), error.offset()),
            (kind, offset),
            "input {:?}",
            String::from_utf8_lossy(input)
        ),
    }
}

/// Asserts that `input` canonicalizes to exactly `expected`.
#[track_caller]
fn canonical(input: &[u8], expected: &[u8]) {
    match canonicalize(input) {
        Ok(bytes) => assert_eq!(
            String::from_utf8_lossy(&bytes),
            String::from_utf8_lossy(expected),
            "input {:?}",
            String::from_utf8_lossy(input)
        ),
        Err(error) => panic!(
            "{:?} was refused with {error}",
            String::from_utf8_lossy(input)
        ),
    }
}

#[test]
fn codes_are_stable() {
    let codes = [
        (CanonErrorKind::InputTooLarge, "input_too_large"),
        (CanonErrorKind::DepthExceeded, "depth_exceeded"),
        (CanonErrorKind::InvalidUtf8, "invalid_utf8"),
        (CanonErrorKind::LoneSurrogate, "lone_surrogate"),
        (CanonErrorKind::DuplicateKey, "duplicate_key"),
        (CanonErrorKind::NumberNotInteger, "number_not_integer"),
        (CanonErrorKind::NumberOutOfRange, "number_out_of_range"),
        (CanonErrorKind::JsonSyntax, "json_syntax"),
    ];
    for (kind, code) in codes {
        assert_eq!(kind.code(), code);
    }
}

#[test]
fn number_not_integer_refuses_fractions_exponents_and_negative_zero() {
    for input in ["1.0", "1e2", "1E0", "100E-2", "-0", "0.5", "-1.5e+3", "0e0"] {
        refused(input.as_bytes(), CanonErrorKind::NumberNotInteger, 0);
    }
    // Reported at the token's first byte, not where the fraction starts.
    refused(b"[7, 12.00]", CanonErrorKind::NumberNotInteger, 4);
    refused(b"{\"a\":-0}", CanonErrorKind::NumberNotInteger, 5);
}

#[test]
fn number_out_of_range_refuses_two_pow_53() {
    refused(b"9007199254740992", CanonErrorKind::NumberOutOfRange, 0);
    refused(b"-9007199254740992", CanonErrorKind::NumberOutOfRange, 0);
    refused(b"[1,9007199254740993]", CanonErrorKind::NumberOutOfRange, 3);
    // Past i64 and past u64: the digits must not wrap around into range.
    refused(b"9223372036854775808", CanonErrorKind::NumberOutOfRange, 0);
    refused(
        b"100000000000000000000000000000",
        CanonErrorKind::NumberOutOfRange,
        0,
    );
    refused(
        b"-18446744073709551617",
        CanonErrorKind::NumberOutOfRange,
        0,
    );
}

#[test]
fn integers_within_two_pow_53_minus_one_are_accepted() {
    canonical(b"9007199254740991", b"9007199254740991");
    canonical(b"-9007199254740991", b"-9007199254740991");
    canonical(b"0", b"0");
    canonical(b"[ -1 , 42 ]", b"[-1,42]");
    assert_eq!(
        Integer::new(9_007_199_254_740_991).map(Integer::get),
        Some(Integer::MAX)
    );
    assert_eq!(
        Integer::new(-9_007_199_254_740_991).map(Integer::get),
        Some(Integer::MIN)
    );
    assert_eq!(Integer::new(9_007_199_254_740_992), None);
    assert_eq!(Integer::new(-9_007_199_254_740_992), None);
}

#[test]
fn duplicate_key_refuses_keys_equal_after_decoding() {
    // `\u0061` decodes to `a`; the fault is at the second key's quote.
    refused(br#"{"a":1,"\u0061":2}"#, CanonErrorKind::DuplicateKey, 7);
    // `\u00e9` and a raw U+00E9 decode to the same key.
    refused(
        "{\"\\u00e9\":1,\"\u{e9}\":2}".as_bytes(),
        CanonErrorKind::DuplicateKey,
        12,
    );
    // `\/` and `/` decode to the same key.
    refused(br#"{"\/":1,"/":2}"#, CanonErrorKind::DuplicateKey, 8);
    // Plain duplicates, also inside nested objects.
    refused(br#"{"a":1,"a":1}"#, CanonErrorKind::DuplicateKey, 7);
    refused(
        br#"[{"x":{"k":1,"j":2,"k":3}}]"#,
        CanonErrorKind::DuplicateKey,
        19,
    );
    // Equal keys in different objects are fine.
    canonical(
        br#"{"a":{"a":1},"b":{"a":2}}"#,
        br#"{"a":{"a":1},"b":{"a":2}}"#,
    );
}

#[test]
fn lone_surrogate_refuses_unpaired_high_and_low_surrogates() {
    refused(br#""\ud800""#, CanonErrorKind::LoneSurrogate, 1);
    refused(br#""\udc00""#, CanonErrorKind::LoneSurrogate, 1);
    refused(br#""\uDBFF""#, CanonErrorKind::LoneSurrogate, 1);
    refused(br#""\uDFFF""#, CanonErrorKind::LoneSurrogate, 1);
    refused(br#""ab\ud800x""#, CanonErrorKind::LoneSurrogate, 3);
    // A high surrogate followed by a non-surrogate escape or another high one.
    refused(br#""\ud800\u0041""#, CanonErrorKind::LoneSurrogate, 1);
    refused(br#""\ud800\ud800""#, CanonErrorKind::LoneSurrogate, 1);
    // Low then high is two lone surrogates; the first is reported.
    refused(br#""\udc00\ud800""#, CanonErrorKind::LoneSurrogate, 1);
    // A lone surrogate in a key.
    refused(br#"{"\udfff":1}"#, CanonErrorKind::LoneSurrogate, 2);
    // A high surrogate cut off by the end of the input is still reported at
    // its backslash, the lowest offset.
    refused(br#""\ud800"#, CanonErrorKind::LoneSurrogate, 1);
}

#[test]
fn invalid_utf8_refuses_ill_formed_bytes() {
    // A raw invalid byte inside a string.
    refused(b"\"\xff\"", CanonErrorKind::InvalidUtf8, 1);
    // A raw invalid byte outside a string.
    refused(b"[1,\xff]", CanonErrorKind::InvalidUtf8, 3);
    refused(b"\xfe", CanonErrorKind::InvalidUtf8, 0);
    // Overlong encoding, encoded surrogate, past U+10FFFF, lone continuation.
    refused(b"\"\xc0\x80\"", CanonErrorKind::InvalidUtf8, 1);
    refused(b"\"\xed\xa0\x80\"", CanonErrorKind::InvalidUtf8, 1);
    refused(b"\"\xf4\x90\x80\x80\"", CanonErrorKind::InvalidUtf8, 1);
    refused(b"\"a\x80\"", CanonErrorKind::InvalidUtf8, 2);
    // A sequence cut short by the closing quote or the end of the input.
    refused(b"\"a\xe2\x82\"", CanonErrorKind::InvalidUtf8, 2);
    refused(b"\"\xf0\x9f\x98", CanonErrorKind::InvalidUtf8, 1);
}

#[test]
fn json_syntax_refuses_grammar_errors() {
    let cases: &[(&[u8], usize)] = &[
        (b"NaN", 0),
        (b"[Infinity]", 1),
        (b"01", 1),
        (b"[00]", 2),
        (b"+1", 0),
        (b"[+1]", 1),
        (b"[1,]", 3),
        (b"{\"a\":1,}", 7),
        (b"1 2", 2),
        (b"{}x", 2),
        (b"", 0),
        (b"   ", 3),
        (b"[", 1),
        (b"{\"a\" 1}", 5),
        (b"{a:1}", 1),
        (b"\"a", 2),
        (b"\"\x01\"", 1),
        (b"\"\\x\"", 2),
        (b"\"\\u12G4\"", 5),
        (b"nul", 3),
        (b"True", 0),
        // A byte-order mark is not JSON.
        (b"\xef\xbb\xbf1", 0),
        // Valid UTF-8 that is not ASCII is a grammar error outside strings.
        ("[\u{e9}]".as_bytes(), 1),
    ];
    for &(input, offset) in cases {
        refused(input, CanonErrorKind::JsonSyntax, offset);
    }
}

#[test]
fn json_syntax_refuses_tokens_that_are_not_rfc_8259_numbers() {
    // At the first byte the number grammar cannot accept.
    refused(b"1.", CanonErrorKind::JsonSyntax, 2);
    refused(b"1e", CanonErrorKind::JsonSyntax, 2);
    refused(b"1e+", CanonErrorKind::JsonSyntax, 3);
    refused(b"-", CanonErrorKind::JsonSyntax, 1);
    refused(b"-01", CanonErrorKind::JsonSyntax, 2);
    refused(b"[-01]", CanonErrorKind::JsonSyntax, 3);
    refused(b".5", CanonErrorKind::JsonSyntax, 0);
    refused(b"1.e5", CanonErrorKind::JsonSyntax, 2);
    refused(b"[1.]", CanonErrorKind::JsonSyntax, 3);
    refused(b"1-2", CanonErrorKind::JsonSyntax, 1);
    refused(b"1.0.0", CanonErrorKind::JsonSyntax, 3);
}

#[test]
fn input_too_large_is_refused_one_byte_past_one_mebibyte() {
    // A string document of exactly 1,048,576 bytes is accepted unchanged.
    let mut at_limit = vec![b'a'; MAX_INPUT_BYTES];
    at_limit[0] = b'"';
    at_limit[MAX_INPUT_BYTES - 1] = b'"';
    assert_eq!(at_limit.len(), 1_048_576);
    assert_eq!(canonicalize(&at_limit).as_deref(), Ok(at_limit.as_slice()));

    // One byte more, still valid JSON, is refused.
    let mut past_limit = at_limit.clone();
    past_limit.push(b' ');
    assert_eq!(past_limit.len(), 1_048_577);
    refused(&past_limit, CanonErrorKind::InputTooLarge, MAX_INPUT_BYTES);

    // The size check comes before every other fault, at any offset.
    let garbage = vec![0xff; 1_048_577];
    refused(&garbage, CanonErrorKind::InputTooLarge, MAX_INPUT_BYTES);
}

#[test]
fn depth_exceeded_is_refused_at_the_65th_bracket() {
    assert_eq!(MAX_DEPTH, 64);
    let nested = |depth: usize| {
        let mut input = "[".repeat(depth).into_bytes();
        input.extend("]".repeat(depth).into_bytes());
        input
    };
    let depth_64 = nested(64);
    canonical(&depth_64, &depth_64);
    refused(&nested(65), CanonErrorKind::DepthExceeded, 64);

    // Objects count the same way: 63 arrays plus one object is depth 64.
    let mut mixed = "[".repeat(63).into_bytes();
    mixed.extend_from_slice(b"{}");
    mixed.extend("]".repeat(63).into_bytes());
    canonical(&mixed, &mixed);
    let mut too_deep = "{\"a\":".repeat(64).into_bytes();
    too_deep.extend_from_slice(b"[]");
    too_deep.extend("}".repeat(64).into_bytes());
    refused(&too_deep, CanonErrorKind::DepthExceeded, 64 * 5);
}

#[test]
fn several_faults_report_the_lowest_offset() {
    // A duplicate key before a number fault reports the duplicate key ...
    refused(br#"{"a":1,"a":1.0}"#, CanonErrorKind::DuplicateKey, 7);
    // ... and a number fault before a duplicate key reports the number.
    refused(br#"{"a":1.0,"a":2}"#, CanonErrorKind::NumberNotInteger, 5);
    // At one offset, invalid_utf8 wins over json_syntax: a raw invalid byte
    // where the grammar expects a value, a separator or the end.
    refused(b"[\xff]", CanonErrorKind::InvalidUtf8, 1);
    refused(b"[1\xff]", CanonErrorKind::InvalidUtf8, 2);
    refused(b"1.\xff", CanonErrorKind::InvalidUtf8, 2);
    refused(b"1 \xc3", CanonErrorKind::InvalidUtf8, 2);
    // Earlier faults of other kinds win over a later invalid byte ...
    refused(b"[1.0,\xff]", CanonErrorKind::NumberNotInteger, 1);
    refused(b"[x,\xff]", CanonErrorKind::JsonSyntax, 1);
    refused(b"[\"\\ud800\",\"\xff\"]", CanonErrorKind::LoneSurrogate, 2);
    // ... and an earlier invalid byte wins over later faults.
    refused(b"[\"\xff\",1.0]", CanonErrorKind::InvalidUtf8, 2);
    refused(b"[\"\xff\\ud800\"]", CanonErrorKind::InvalidUtf8, 2);
    // A fault inside the value of a key comes before a later duplicate.
    refused(br#"{"a":"\udc00","a":1}"#, CanonErrorKind::LoneSurrogate, 6);
    // A duplicate key comes before a syntax error after it.
    refused(br#"{"a":1,"a" 2}"#, CanonErrorKind::DuplicateKey, 7);
}

#[test]
fn key_order_puts_u10000_before_ue000() {
    // U+10000 is the UTF-16 pair D800 DC00, which sorts before E000, though
    // its code point and its UTF-8 bytes sort after.
    canonical(
        br#"{"\ue000":1,"\ud800\udc00":2}"#,
        "{\"\u{10000}\":2,\"\u{e000}\":1}".as_bytes(),
    );
}

#[test]
fn rfc_8785_sorting_example() {
    // RFC 8785 §3.2.3.
    let input = br#"{
   "\u20ac": "Euro Sign",
   "\r": "Carriage Return",
   "\ufb33": "Hebrew Letter Dalet With Dagesh",
   "1": "One",
   "\ud83d\ude00": "Emoji: Grinning Face",
   "\u0080": "Control",
   "\u00f6": "Latin Small Letter O With Diaeresis"
}"#;
    let expected = concat!(
        "{",
        "\"\\r\":\"Carriage Return\",",
        "\"1\":\"One\",",
        "\"\u{80}\":\"Control\",",
        "\"\u{f6}\":\"Latin Small Letter O With Diaeresis\",",
        "\"\u{20ac}\":\"Euro Sign\",",
        "\"\u{1f600}\":\"Emoji: Grinning Face\",",
        "\"\u{fb33}\":\"Hebrew Letter Dalet With Dagesh\"",
        "}"
    );
    canonical(input, expected.as_bytes());
}

/// Input and expected canonical bytes, written by hand.
const HAND_WRITTEN: &[(&str, &str)] = &[
    // Every control character U+0000–U+001F: the five with short forms use
    // them, the rest are `\u00xx` in lowercase hex, whatever case the input
    // used.
    (r#""\u0000""#, r#""\u0000""#),
    (r#""\u0001""#, r#""\u0001""#),
    (r#""\u0002""#, r#""\u0002""#),
    (r#""\u0003""#, r#""\u0003""#),
    (r#""\u0004""#, r#""\u0004""#),
    (r#""\u0005""#, r#""\u0005""#),
    (r#""\u0006""#, r#""\u0006""#),
    (r#""\u0007""#, r#""\u0007""#),
    (r#""\u0008""#, r#""\b""#),
    (r#""\b""#, r#""\b""#),
    (r#""\u0009""#, r#""\t""#),
    (r#""\t""#, r#""\t""#),
    (r#""\u000A""#, r#""\n""#),
    (r#""\n""#, r#""\n""#),
    (r#""\u000B""#, r#""\u000b""#),
    (r#""\u000C""#, r#""\f""#),
    (r#""\f""#, r#""\f""#),
    (r#""\u000D""#, r#""\r""#),
    (r#""\r""#, r#""\r""#),
    (r#""\u000E""#, r#""\u000e""#),
    (r#""\u000F""#, r#""\u000f""#),
    (r#""\u0010""#, r#""\u0010""#),
    (r#""\u0011""#, r#""\u0011""#),
    (r#""\u0012""#, r#""\u0012""#),
    (r#""\u0013""#, r#""\u0013""#),
    (r#""\u0014""#, r#""\u0014""#),
    (r#""\u0015""#, r#""\u0015""#),
    (r#""\u0016""#, r#""\u0016""#),
    (r#""\u0017""#, r#""\u0017""#),
    (r#""\u0018""#, r#""\u0018""#),
    (r#""\u0019""#, r#""\u0019""#),
    (r#""\u001a""#, r#""\u001a""#),
    (r#""\u001B""#, r#""\u001b""#),
    (r#""\u001c""#, r#""\u001c""#),
    (r#""\u001D""#, r#""\u001d""#),
    (r#""\u001e""#, r#""\u001e""#),
    (r#""\u001F""#, r#""\u001f""#),
    // U+007F, `/`, U+2028, U+2029 and non-ASCII are written raw.
    (r#""\u007f""#, "\"\u{7f}\""),
    ("\"\u{7f}\"", "\"\u{7f}\""),
    (r#""/""#, r#""/""#),
    (r#""\u2028""#, "\"\u{2028}\""),
    (r#""\u2029""#, "\"\u{2029}\""),
    ("\"\u{2028}\u{2029}\"", "\"\u{2028}\u{2029}\""),
    (r#""\u00e9""#, "\"\u{e9}\""),
    (
        "\"caf\u{e9} \u{65e5}\u{672c}\"",
        "\"caf\u{e9} \u{65e5}\u{672c}\"",
    ),
    (r#""\ud83d\ude00""#, "\"\u{1f600}\""),
    (r#""\u0080""#, "\"\u{80}\""),
    // `"` and `\` are always written as `\"` and `\\`, even when the input
    // wrote them as `\u` escapes.
    (r#""\"""#, r#""\"""#),
    (r#""\\""#, r#""\\""#),
    (r#""\u0022""#, r#""\"""#),
    (r#""\u005C""#, r#""\\""#),
    // Escapes that need none are decoded.
    (r#""\u0061""#, r#""a""#),
    (r#""\/""#, r#""/""#),
    (r#""\u0041\u0020\u007E""#, r#""A ~""#),
    // Whitespace outside strings is removed; inside strings it stays.
    (" \t\r\n[ 1 ,\t2 ,\r\n3 ] \n", "[1,2,3]"),
    ("{ \"a b\" : \" x \" }", "{\"a b\":\" x \"}"),
    // Literals and empty containers.
    (
        "[ true , false , null , { } , [ ] ]",
        "[true,false,null,{},[]]",
    ),
    // Nested objects are sorted; arrays keep their order.
    (
        r#"{"b":[3,1,2],"a":{"z":null,"y":[{"d":1,"c":2}]}}"#,
        r#"{"a":{"y":[{"c":2,"d":1}],"z":null},"b":[3,1,2]}"#,
    ),
    (r#"[{"b":1,"a":2},{"a":3}]"#, r#"[{"a":2,"b":1},{"a":3}]"#),
    // Keys that differ only beyond the Basic Multilingual Plane, both in the
    // high and in the low surrogate.
    (
        r#"{"\ud83d\ude01":1,"\ud83d\ude00":2}"#,
        "{\"\u{1f600}\":2,\"\u{1f601}\":1}",
    ),
    (
        r#"{"\ud83e\udd00":1,"\ud83d\udfff":2}"#,
        "{\"\u{1f7ff}\":2,\"\u{1f900}\":1}",
    ),
    // Keys are sorted by code unit, so a prefix comes first and case matters.
    (
        r#"{"ab":1,"a":2,"B":3,"b":4}"#,
        r#"{"B":3,"a":2,"ab":1,"b":4}"#,
    ),
];

#[test]
fn hand_written_escape_and_ordering_table() {
    for &(input, expected) in HAND_WRITTEN {
        canonical(input.as_bytes(), expected.as_bytes());
    }
}

#[test]
fn canonical_output_is_a_fixed_point() {
    for &(_, expected) in HAND_WRITTEN {
        canonical(expected.as_bytes(), expected.as_bytes());
    }
}

#[test]
fn values_built_in_code_write_the_same_bytes_as_parsed_ones() {
    let mut inner = Object::new();
    inner.insert("z", Value::Null);
    inner.insert("\u{10000}", Value::Bool(true));
    inner.insert("\u{e000}", Value::Bool(false));
    let mut root = Object::new();
    root.insert("list", Value::Array(vec![Value::String("\n/".into())]));
    root.insert("count", Value::Integer(Integer::new(-3).unwrap()));
    root.insert("inner", Value::Object(inner));
    assert_eq!(
        root.insert("count", Value::Null),
        Some(Value::Integer(Integer::new(-3).unwrap()))
    );
    let built = Value::Object(root);

    let text = "{\"count\":null,\"inner\":{\"\u{10000}\":true,\"z\":null,\"\u{e000}\":false},\"list\":[\"\\n/\"]}";
    let parsed = parse(text.as_bytes()).unwrap();
    assert_eq!(built, parsed);
    let expected = "{\"count\":null,\"inner\":{\"z\":null,\"\u{10000}\":true,\"\u{e000}\":false},\"list\":[\"\\n/\"]}";
    assert_eq!(String::from_utf8(built.to_canonical()).unwrap(), expected);
    if let Value::Object(object) = &parsed {
        assert_eq!(object.len(), 3);
        assert_eq!(object.get("count"), Some(&Value::Null));
        let keys: Vec<&str> = object.iter().map(|(key, _)| key).collect();
        assert_eq!(keys, ["count", "inner", "list"]);
    } else {
        panic!("the root parsed as {parsed:?}");
    }
}

#[test]
fn committed_canonical_vectors_agree_with_the_hand_written_table() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-vectors/rich-ui-registry-v1");
    let inputs: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("canon-")
        })
        .filter(|path| path.to_str().unwrap().ends_with(".input.json"))
        .collect();
    for &(input, expected) in HAND_WRITTEN {
        let value = parse(input.as_bytes()).unwrap();
        let mut matched = 0;
        for path in &inputs {
            let bytes = std::fs::read(path).unwrap();
            if parse(&bytes).unwrap() == value {
                let canonical_path = path.with_file_name(
                    path.file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace(".input.json", ".canonical.json"),
                );
                assert_eq!(
                    std::fs::read(canonical_path).unwrap(),
                    expected.as_bytes(),
                    "{}",
                    path.display()
                );
                matched += 1;
            }
        }
        assert!(matched > 0, "no committed vector for {input:?}");
    }
}
