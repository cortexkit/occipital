use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(untagged)]
enum Text {
    Literal(String),
    Repeated {
        prefix: String,
        unit: String,
        count: usize,
        suffix: String,
    },
}

impl Text {
    fn expand(&self) -> String {
        match self {
            Self::Literal(s) => s.clone(),
            Self::Repeated {
                prefix,
                unit,
                count,
                suffix,
            } => {
                format!("{prefix}{}{suffix}", unit.repeat(*count))
            }
        }
    }
}

#[derive(Deserialize)]
struct Vector {
    name: String,
    rule: String,
    derivation: String,
    input: Text,
    canonical: Option<Text>,
    whatwg: Option<Text>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct DisplayVector {
    input: String,
    expected: String,
}

#[derive(Deserialize)]
struct Vectors {
    addresses: Vec<Vector>,
    display: Vec<DisplayVector>,
}

fn vectors() -> Vectors {
    serde_json::from_str(include_str!("../../tests/vectors/urls.json")).unwrap()
}

fn check_rule(rule: &str) {
    let mut count = 0;
    for vector in vectors().addresses.iter().filter(|v| v.rule == rule) {
        count += 1;
        assert!(!vector.derivation.is_empty(), "{}", vector.name);
        let input = vector.input.expand();
        let actual = Address::parse(&input);
        match &vector.canonical {
            Some(expected) => {
                let actual = actual.unwrap_or_else(|e| panic!("{}: {e:?}", vector.name));
                let expected = expected.expand();
                assert_eq!(actual.as_str(), expected, "{}", vector.name);
                assert_eq!(
                    actual.as_str(),
                    vector.whatwg.as_ref().unwrap().expand(),
                    "WHATWG reference: {}",
                    vector.name
                );
                assert!(vector.error.is_none());
                assert_eq!(
                    Address::parse(actual.as_str()),
                    Ok(actual.clone()),
                    "canonical fixed point: {}",
                    vector.name
                );
                assert_eq!(
                    actual.display().as_str(),
                    expected,
                    "full sheet: {}",
                    vector.name
                );
                assert_eq!(serde_json::to_value(&actual).unwrap(), expected);
            }
            None => {
                assert_eq!(actual, Err(DiagnosticCode::Scheme), "{}", vector.name);
                assert_eq!(vector.error.as_deref(), Some("E_SCHEME"));
                assert!(vector.whatwg.is_none());
            }
        }
    }
    assert!(count > 0, "no vectors for {rule}");
}

macro_rules! rule_test {
    ($test:ident, $rule:literal) => {
        #[test]
        fn $test() {
            check_rule($rule);
        }
    };
}

rule_test!(scheme_and_absolute, "scheme");
rule_test!(idna_mapping, "mapping");
rule_test!(idna_nfc, "nfc");
rule_test!(idna_bidi, "bidi");
rule_test!(idna_joiners, "joiners");
rule_test!(idna_hyphens, "hyphens");
rule_test!(idna_std3, "std3");
rule_test!(idna_dns_length, "dns");
rule_test!(host_refusals, "host");
rule_test!(userinfo_refused, "userinfo");
rule_test!(ports, "port");
rule_test!(escapes, "escape");
rule_test!(encoding, "encoding");
rule_test!(paths, "path");
rule_test!(raw_characters, "raw");
rule_test!(scalar_caps, "length");

#[test]
fn display_escaping() {
    for vector in vectors().display {
        assert_eq!(
            DisplayText::address_sheet(&vector.input).as_str(),
            vector.expected
        );
    }
}

macro_rules! checksum {
    ($name:ident, $file:literal, $sha:literal) => {
        #[test]
        fn $name() {
            let digest = hmac_sha256::Hash::hash(include_bytes!($file));
            let actual: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(actual, $sha, "Unicode 16 table changed: {}", $file);
        }
    };
}

checksum!(
    mapping_sha256,
    "tables/mapping.rs",
    "97f6010a7467080814618c4eefa93bbdb2412ede337847b01db6574ff8f8cd20"
);
checksum!(
    nfc_sha256,
    "tables/nfc.rs",
    "533efe70952ea55d780b94953e41ece5e95cdda4f9313b38b745a0c0cd39cdd2"
);
checksum!(
    bidi_sha256,
    "tables/bidi.rs",
    "e513152bebb1f4188aacf81605cc6e558be2527da6acfe275fc27eb8382dec0a"
);
checksum!(
    joining_sha256,
    "tables/joining.rs",
    "c2f9a879397b532996c7702475b1adfc8a026561bbd4ca363d88ccee6d6e1e6a"
);

#[test]
fn normalization_composition_order_and_blocking() {
    for (input, expected) in [
        ("A\u{30A}", "Å"),
        ("\u{212B}", "Å"),
        ("a\u{315}\u{300}", "à\u{315}"),
        ("a\u{305}\u{301}", "a\u{305}\u{301}"),
        ("\u{1100}\u{1161}\u{11A8}", "각"),
        ("\u{AC00}\u{11A8}", "각"),
        ("\u{344}", "\u{308}\u{301}"),
        ("\u{301}\u{323}x", "\u{323}\u{301}x"),
    ] {
        assert_eq!(normalize::nfc(input), expected);
        assert_eq!(normalize::nfc(expected), expected);
    }
}

#[test]
fn punycode_rfc3492_and_invalid_scalars() {
    for (unicode, ascii) in [
        ("bücher", "bcher-kva"),
        ("mañana", "maana-pta"),
        ("例え", "r8jz45g"),
        ("ليهمابتكلموشعربي؟", "egbpdaj6bu4bxfgehfvwxn"),
        ("他们为什么不说中文", "ihqwcrb4cv8a8dqg056pqjye"),
    ] {
        assert_eq!(punycode::encode(unicode).as_deref(), Some(ascii));
        assert_eq!(punycode::decode(ascii).as_deref(), Some(unicode));
    }
    for invalid in ["0", "!", "é", "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"] {
        assert_eq!(punycode::decode(invalid), None, "{invalid}");
    }
}

#[test]
fn tables_are_sorted_and_cover_the_mapping_space() {
    let mut end = 0;
    for &(lo, hi, status, _) in tables::mapping::ROWS {
        assert_eq!(lo, end);
        assert!(hi >= lo && status <= 4);
        end = hi + 1;
    }
    assert_eq!(end, 0x110000);
    for ranges in [tables::bidi::RANGES, tables::joining::RANGES] {
        for pair in ranges.windows(2) {
            assert!(pair[0].0 <= pair[0].1 && pair[0].1 < pair[1].0);
        }
    }
    assert!(tables::nfc::DECOMPOSITION
        .windows(2)
        .all(|p| p[0].0 < p[1].0));
    assert!(tables::nfc::COMPOSITION.windows(2).all(|p| p[0].0 < p[1].0));
}
