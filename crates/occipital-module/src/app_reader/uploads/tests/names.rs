//! The name rules are pure, so they run on every platform: a name cleaned on Linux
//! must also be valid on Windows and macOS.

use super::destination::{candidate, clean_name, fold, is_reserved, utc_date, MAX_NAME_BYTES};

#[test]
fn names_keep_only_their_last_component() {
    assert_eq!(clean_name("../../etc/passwd"), "passwd");
    assert_eq!(clean_name(r"C:\Users\someone\clip.mov"), "clip.mov");
    assert_eq!(clean_name("dir/"), "upload");
}

#[test]
fn characters_windows_refuses_and_control_characters_are_removed() {
    assert_eq!(clean_name("a<b>c:d\"e|f?g*h.txt"), "abcdefgh.txt");
    assert_eq!(clean_name("a\0b\u{7}c\u{1b}\n.txt"), "abc.txt");
    for name in ["<>:\"|?*", "\0", "\u{1}\u{2}"] {
        assert_eq!(clean_name(name), "upload", "{name:?}");
    }
}

#[test]
fn leading_dots_and_trailing_dots_and_spaces_are_removed() {
    assert_eq!(clean_name(".bashrc"), "bashrc");
    assert_eq!(clean_name("..hidden.txt"), "hidden.txt");
    assert_eq!(clean_name("  report.pdf . . "), "report.pdf");
    assert_eq!(clean_name("name..."), "name");
    for name in [".", "..", "...", " . ", ""] {
        assert_eq!(clean_name(name), "upload", "{name:?}");
    }
}

#[test]
fn windows_device_names_are_never_used_whatever_their_case_or_extension() {
    for name in [
        "CON",
        "con",
        "Prn",
        "aux.txt",
        "NUL.tar.gz",
        "com1",
        "COM9.log",
        "lpt1",
        "Lpt9.x",
        "CON .txt",
    ] {
        assert!(is_reserved(name), "{name}");
        let cleaned = clean_name(name);
        assert!(!is_reserved(&cleaned), "{name} -> {cleaned}");
        assert!(cleaned.starts_with('_'), "{name} -> {cleaned}");
    }
    assert_eq!(clean_name("nul.tar.gz"), "_nul.tar.gz");
    for name in [
        "COM10",
        "LPT0.txt",
        "console.log",
        "auxiliary",
        "nullable.rs",
        "CONN",
    ] {
        assert!(!is_reserved(name), "{name}");
        assert_eq!(clean_name(name), name);
    }
}

#[test]
fn long_names_are_bounded_and_keep_their_extension() {
    let long = format!("{}.mov", "a".repeat(400));
    let cleaned = clean_name(&long);
    assert_eq!(cleaned.len(), MAX_NAME_BYTES);
    assert!(cleaned.ends_with(".mov"));
    // Multi-byte characters are never cut in half.
    let wide = "é".repeat(300);
    let cleaned = clean_name(&wide);
    assert!(cleaned.len() <= MAX_NAME_BYTES);
    assert!(cleaned.chars().all(|c| c == 'é'));
    // A cut that ends on a dot or space doesn't leave one at the end.
    let dotted = format!("{}. x", "b".repeat(MAX_NAME_BYTES - 1));
    let cleaned = clean_name(&dotted);
    assert!(!cleaned.ends_with(['.', ' ']), "{cleaned:?}");
    // Every suffixed form fits the file systems' 255-byte limit.
    assert!(candidate(&clean_name(&long), 10_000).len() <= 255);
}

#[test]
fn taken_names_are_suffixed_before_the_extension_and_compared_without_case() {
    assert_eq!(candidate("clip.mov", 1), "clip.mov");
    assert_eq!(candidate("clip.mov", 2), "clip-2.mov");
    assert_eq!(candidate("archive.tar.gz", 3), "archive.tar-3.gz");
    assert_eq!(candidate("README", 2), "README-2");
    assert_eq!(fold("Clip.MOV"), fold("clip.mov"));
    assert_eq!(fold("ÉTÉ.txt"), fold("été.txt"));
}

#[test]
fn dates_are_utc_calendar_days() {
    assert_eq!(utc_date(0), "1970-01-01");
    assert_eq!(utc_date(1_767_225_600_000), "2026-01-01");
    assert_eq!(utc_date(1_767_225_600_000 - 1), "2025-12-31");
    assert_eq!(utc_date(951_782_400_000), "2000-02-29");
    assert_eq!(utc_date(-1), "1969-12-31");
}
