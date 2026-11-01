//! Replays the public interchange files and checks the example's writer, not a
//! second fixture serializer. Checksums use a separate test-only SHA-256 below.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use occipital_registry::{canon, Template};

#[path = "../examples/registry_vectors.rs"]
mod example;

fn directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("test-vectors/rich-ui-registry-v1")
}

fn files(directory: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            assert!(entry.file_type().unwrap().is_file());
            (
                entry.file_name().into_string().unwrap(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn example_program_reproduces_committed_vectors_byte_for_byte() {
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/scratch/registry-vectors-{}",
        std::process::id()
    ));
    assert!(
        !output.exists(),
        "fresh output directory required: {}",
        output.display()
    );
    // main delegates directly to this writer; exercising it also checks the
    // filesystem output, including binary refusal inputs and final newlines.
    let count = example::write_vectors(&output).unwrap();
    let expected = files(&directory());
    let actual = files(&output);
    assert_eq!(count, expected.len());
    assert_eq!(
        actual.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>()
    );
    for (path, bytes) in expected {
        assert_eq!(actual[&path], bytes, "example changed {path}");
    }
    std::fs::remove_dir_all(output).unwrap();
}

#[test]
fn sha256sums_format_coverage_and_independent_hashes() {
    assert_eq!(
        sha256(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let mut files = files(&directory());
    let sums = String::from_utf8(files.remove("SHA256SUMS").unwrap()).unwrap();
    assert!(sums.ends_with('\n'));
    assert!(!sums.contains('\r'));
    let mut previous = "";
    let mut seen = BTreeSet::new();
    for line in sums.strip_suffix('\n').unwrap().split('\n') {
        let (hex, path) = line.split_once("  ").expect("two-space separator");
        assert_eq!(hex.len(), 64, "{line}");
        assert!(hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert!(!path.is_empty() && !path.contains(['\\', ' ', '\t']));
        assert!(!path.starts_with('/') && path.split('/').all(|p| !matches!(p, "" | "." | "..")));
        assert!(
            previous.as_bytes() < path.as_bytes(),
            "paths must be bytewise sorted and unique"
        );
        previous = path;
        assert!(seen.insert(path.to_owned()));
        assert_eq!(
            hex,
            sha256(files.get(path).expect("listed file exists")),
            "{path}"
        );
    }
    assert_eq!(seen, files.keys().cloned().collect());
}

fn canonical(input: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let value = canon::parse(input).map_err(|e| e.code().to_owned())?;
    let schema = match &value {
        canon::Value::Object(object) => match object.get("schema") {
            Some(canon::Value::String(schema)) => schema.as_str(),
            _ => "",
        },
        _ => "",
    };
    if schema.starts_with("occipital.template/")
        || name.starts_with("template-")
        || name.starts_with("shape-template-")
        || name.starts_with("precedence-template-")
    {
        Template::parse(input)
            .and_then(|t| t.to_canonical())
            .map_err(|e| e.code().into())
    } else {
        Ok(value.to_canonical())
    }
}

#[test]
fn committed_vectors_replay_through_the_public_api() {
    let files = files(&directory());
    let mut canonical_count = 0;
    let mut refused_count = 0;
    for (path, input) in &files {
        if let Some(name) = path.strip_suffix(".input.json") {
            let bytes = canonical(input, name).unwrap_or_else(|code| panic!("{name}: {code}"));
            assert_eq!(bytes, files[&format!("{name}.canonical.json")], "{name}");
            assert_eq!(
                format!("sha256:{}\n", sha256(&bytes)).as_bytes(),
                files[&format!("{name}.digest")],
                "{name}"
            );
            canonical_count += 1;
        } else if let Some(name) = path.strip_suffix(".input") {
            let code = canonical(input, name).expect_err(name);
            assert_eq!(
                format!("{code}\n").as_bytes(),
                files[&format!("{name}.code")],
                "{name}"
            );
            refused_count += 1;
        }
    }
    // Exact, so a case that silently stops being generated fails here.
    assert_eq!((canonical_count, refused_count), (71, 47));
}

// A compact reference implementation kept separate from hmac-sha256 and the
// generator, so hashing a filename, omitting bytes or writing a bad hex digest
// cannot make a self-consistent but invalid checksum manifest pass.
fn sha256(input: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut padded = input.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&((input.len() as u64) * 8).to_be_bytes());
    for block in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
    state.iter().map(|word| format!("{word:08x}")).collect()
}
