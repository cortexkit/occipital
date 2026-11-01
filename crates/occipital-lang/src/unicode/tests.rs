use super::*;

macro_rules! checksum_test {
    ($test:ident, $path:literal, $expected:literal) => {
        #[test]
        fn $test() {
            let digest = hmac_sha256::Hash::hash(include_bytes!($path));
            let actual: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(actual, $expected, "Unicode 16.0 table changed: {}", $path);
        }
    };
}

checksum_test!(
    upper_table_sha256_is_pinned,
    "upper.rs",
    "35b7f6ce84b20dc8cc2845677e46dc07d728e883f90d86dd6fa3f6eeecfee210"
);
checksum_test!(
    lower_table_sha256_is_pinned,
    "lower.rs",
    "968970df55eedddf41001a93ae9976f3c1a3507a256df32728816aa56a7d9aab"
);
checksum_test!(
    white_space_table_sha256_is_pinned,
    "white_space.rs",
    "16c6cf974ab0afb59aaf6ea1d0ba0c1aea6f412ab0ea32f93139853d26367af6"
);
checksum_test!(
    bidi_control_table_sha256_is_pinned,
    "bidi_control.rs",
    "3e6c9d9c00495b7480b19b07c25754b4113e544039320d426915bcdc0d107212"
);
checksum_test!(
    default_ignorable_table_sha256_is_pinned,
    "default_ignorable.rs",
    "74a627e6bd42d8a0866c096aa5d3d6989be922604245e74283557ae1d3e04e3f"
);
checksum_test!(
    cased_table_sha256_is_pinned,
    "cased.rs",
    "811b5ca1b3fcd37d843268339811d4461865b25f080b96898266d23e85f7e095"
);
checksum_test!(
    case_ignorable_table_sha256_is_pinned,
    "case_ignorable.rs",
    "93b6ed5b5fbf7e580f7e080d4768891096f4788be3b5a49355e8aa291795d806"
);

fn upper(input: &str) -> String {
    case_mapping(input, Case::Upper).collect()
}

fn lower(input: &str) -> String {
    case_mapping(input, Case::Lower).collect()
}

#[test]
fn default_full_case_mapping_golden_vectors() {
    // Expected scalars come from UnicodeData and the unconditional SpecialCasing
    // entries, not from the toolchain's Unicode implementation (§4.3).
    for (input, uppercase, lowercase) in [
        ("", "", ""),
        ("Hello 123!", "HELLO 123!", "hello 123!"),
        ("ß", "SS", "ß"),
        ("ẞ", "ẞ", "ß"),
        ("İ", "İ", "i\u{307}"),
        ("ıiI", "III", "ıii"),
        ("I\u{307}", "I\u{307}", "i\u{307}"),
        ("ﬃ", "FFI", "ﬃ"),
        ("ΐ", "Ι\u{308}\u{301}", "ΐ"),
        ("é e\u{301}", "É E\u{301}", "é e\u{301}"),
        (
            "\u{10400}\u{10428}",
            "\u{10400}\u{10400}",
            "\u{10428}\u{10428}",
        ),
        (
            "\u{180E}\u{200B}😀",
            "\u{180E}\u{200B}😀",
            "\u{180E}\u{200B}😀",
        ),
    ] {
        assert_eq!(upper(input), uppercase, "Upper({input:?})");
        assert_eq!(lower(input), lowercase, "Lower({input:?})");
    }
}

#[test]
fn unicode_16_case_additions_and_version_differences() {
    // These pairs were added in 16.0, so a 15.1 data source misses them.
    for (uppercase, lowercase) in [
        ('\u{1C89}', '\u{1C8A}'),
        ('\u{A7CB}', '\u{264}'),
        ('\u{A7CC}', '\u{A7CD}'),
        ('\u{A7DA}', '\u{A7DB}'),
        ('\u{A7DC}', '\u{19B}'),
        ('\u{10D50}', '\u{10D70}'),
        ('\u{10D65}', '\u{10D85}'),
    ] {
        assert_eq!(upper(&lowercase.to_string()), uppercase.to_string());
        assert_eq!(lower(&uppercase.to_string()), lowercase.to_string());
    }
    // These pairs exist in 17.0, but are unassigned and identity-mapped in 16.0.
    // This catches accidentally replacing pinned data with a newer host version.
    let later = "\u{A7CE}\u{A7CF}\u{16EA0}\u{16EBB}";
    assert_eq!(upper(later), later);
    assert_eq!(lower(later), later);
}

#[test]
fn final_sigma_uses_original_cased_and_case_ignorable_context() {
    for (input, expected) in [
        ("ΟΣ", "ος"),
        ("ΟΣΑ", "οσα"),
        ("Σ", "σ"),
        ("A Σ", "a σ"),
        ("AΣ AΣA", "aς aσa"),
        ("A\u{301}Σ\u{301}", "a\u{301}ς\u{301}"),
        ("AΣ\u{301}B", "aσ\u{301}b"),
        ("A'Σ'", "a'ς'"),
        ("AΣ'B", "aσ'b"),
        ("A.Σ", "a.ς"),
        ("A!Σ", "a!σ"),
        // U+0345 is both Cased and Case_Ignorable; ignorable takes precedence.
        ("\u{345}Σ", "\u{345}σ"),
        ("AΣ\u{345}", "aς\u{345}"),
        // Newly cased Garay letters must also participate in the context rule.
        ("\u{10D50}Σ", "\u{10D70}ς"),
        ("AΣ\u{10D50}", "aσ\u{10D70}"),
    ] {
        assert_eq!(lower(input), expected, "Lower({input:?})");
    }
}

#[test]
fn mapping_can_be_counted_before_any_output_string_is_built() {
    assert_eq!(case_mapping("ßﬃΐ", Case::Upper).count(), 8);
    assert_eq!(case_mapping("İΟΣ", Case::Lower).count(), 4);
}

#[test]
fn white_space_and_trim_golden_vectors() {
    let whitespace = "\t\n\u{B}\u{C}\r \u{85}\u{A0}\u{1680}\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}";
    assert_eq!(whitespace.chars().count(), 25);
    assert!(whitespace.chars().all(is_white_space));
    assert_eq!(trim(whitespace), "");
    assert_eq!(trim(&format!("{whitespace}x y{whitespace}")), "x y");
    // U+180E was White_Space before 6.3; neither it nor U+200B is whitespace
    // in 16.0. Invisible characters must not silently disappear during Trim.
    for c in ['\u{180E}', '\u{200B}', '\u{FEFF}', '\u{200E}', '\u{2060}'] {
        assert!(!is_white_space(c));
        assert_eq!(trim(&format!(" {c}x{c} ")), format!("{c}x{c}"));
    }
    assert_eq!(trim(""), "");
    assert_eq!(trim(" x\u{A0}y "), "x\u{A0}y");
    assert_eq!(
        (0..=0x10FFFF)
            .filter_map(char::from_u32)
            .filter(|c| is_white_space(*c))
            .count(),
        25
    );
}

#[test]
fn display_escaping_covers_controls_bidi_and_default_ignorables() {
    for cp in (0..=0x1F).chain(0x7F..=0x9F) {
        assert!(needs_display_escape(char::from_u32(cp).unwrap()));
    }
    for c in [
        '\u{61C}',
        '\u{200E}',
        '\u{200F}',
        '\u{202A}',
        '\u{202B}',
        '\u{202C}',
        '\u{202D}',
        '\u{202E}',
        '\u{2066}',
        '\u{2067}',
        '\u{2068}',
        '\u{2069}',
        '\u{AD}',
        '\u{34F}',
        '\u{180E}',
        '\u{200B}',
        '\u{200C}',
        '\u{200D}',
        '\u{2060}',
        '\u{FE0F}',
        '\u{FEFF}',
        '\u{E0001}',
        '\u{E0100}',
        '\u{E0FFF}',
    ] {
        assert!(needs_display_escape(c), "U+{:X}", u32::from(c));
    }
    for c in [' ', 'A', 'é', '😀', '\u{A0}', '\u{3000}', '\u{E1000}'] {
        assert!(!needs_display_escape(c), "U+{:X}", u32::from(c));
    }
}

#[test]
fn generated_tables_are_sorted_scalar_mappings_and_disjoint_ranges() {
    for mappings in [upper::MAPPINGS, lower::MAPPINGS] {
        assert!(mappings.windows(2).all(|pair| pair[0].0 < pair[1].0));
        for &(c, mapped) in mappings {
            assert!(!mapped.is_empty());
            assert!(mapped.chars().count() <= 3);
            assert_ne!(c.to_string(), mapped);
        }
    }
    for ranges in [
        white_space::RANGES,
        bidi_control::RANGES,
        default_ignorable::RANGES,
        cased::RANGES,
        case_ignorable::RANGES,
    ] {
        assert!(ranges.windows(2).all(|pair| pair[0].1 + 1 < pair[1].0));
        for &(start, end) in ranges {
            assert!(start <= end);
            assert!(in_ranges(ranges, char::from_u32(start).unwrap()));
            assert!(in_ranges(ranges, char::from_u32(end).unwrap()));
            if let Some(before) = start.checked_sub(1).and_then(char::from_u32) {
                assert!(!in_ranges(ranges, before));
            }
            if let Some(after) = char::from_u32(end + 1) {
                assert!(!in_ranges(ranges, after));
            }
        }
    }
}
