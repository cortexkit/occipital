//! Every pinned contract fixture still holds the exact bytes it was copied
//! with, and every fixture in the directory is pinned.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn pinned_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pinned")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `(sha256, path)` for each line of `PINS`, in order.
fn pins() -> Vec<(String, String)> {
    let text = std::fs::read_to_string(pinned_dir().join("PINS")).expect("read PINS");
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let mut fields = line.split("  ");
            let sha = fields.next().expect("sha256 field").to_owned();
            let path = fields.next().expect("path field").to_owned();
            let source = fields.next().expect("source field");
            assert!(fields.next().is_none(), "three fields: {line}");
            assert!(!source.trim().is_empty(), "a source for {path}");
            assert_eq!(sha.len(), 64, "{line}");
            assert!(sha
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
            (sha, path)
        })
        .collect()
}

fn files_under(dir: &Path, prefix: &str, found: &mut BTreeSet<String>) {
    for entry in std::fs::read_dir(dir).expect("list fixtures") {
        let entry = entry.expect("fixture entry");
        let name = entry.file_name().into_string().expect("UTF-8 fixture name");
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if entry.file_type().expect("file type").is_dir() {
            files_under(&entry.path(), &relative, found);
        } else {
            found.insert(relative);
        }
    }
}

#[test]
fn every_pinned_fixture_matches_its_recorded_sha256() {
    let pins = pins();
    assert!(pins.len() >= 7, "{pins:?}");
    for (sha, path) in &pins {
        let bytes =
            std::fs::read(pinned_dir().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(
            &hex(&hmac_sha256::Hash::hash(&bytes)),
            sha,
            "{path} changed"
        );
    }
}

#[test]
fn every_fixture_in_the_directory_is_pinned() {
    let mut found = BTreeSet::new();
    files_under(&pinned_dir(), "", &mut found);
    found.remove("PINS");
    let pinned: BTreeSet<String> = pins().into_iter().map(|(_, path)| path).collect();
    assert_eq!(found, pinned);
}

#[test]
fn the_sha256_check_can_fail() {
    // The digest above is computed here, so check it against a known value.
    assert_eq!(
        hex(&hmac_sha256::Hash::hash(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
