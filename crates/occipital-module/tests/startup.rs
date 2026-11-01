//! The `ck-occipital` binary's start-up order, driven as the daemon and the
//! placement script drive it: as a separate process with a chosen
//! environment.

use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const NONCE_PREFIX: &str = "ck-occipital: launch nonce unavailable:";

/// A descriptor number nothing in the child has open. The inode is
/// irrelevant: the descriptor is checked for being open first.
const CLOSED_DESCRIPTOR: &str = "987:1";

/// The binary with every launch variable cleared and its data, runtime and
/// config homes pointed into `home`, so a test can tell whether it touched
/// any of them.
fn ck_occipital(home: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ck-occipital"));
    command
        .args(args)
        .stdin(Stdio::null())
        .env_remove("SUBC_LAUNCH_NONCE_FD")
        .env_remove("SUBC_LAUNCH_NONCE_PIPE")
        .env_remove("SUBC_LAUNCH_NONCE_PIPE_FALLBACK")
        .env_remove("SUBC_LAUNCH_NONCE")
        .env_remove("SUBC_MODULE_ID")
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_RUNTIME_DIR", home.join("run"))
        .env("XDG_CONFIG_HOME", home.join("config"));
    command
}

fn run(mut command: Command) -> Output {
    command.output().expect("run the ck-occipital binary")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}

fn assert_nonce_refused(output: &Output) {
    let stderr = stderr(output);
    assert_eq!(output.status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.starts_with(NONCE_PREFIX), "stderr: {stderr}");
    assert_eq!(stderr.lines().count(), 1, "one stderr line: {stderr}");
    assert!(output.stdout.is_empty(), "stdout: {:?}", output.stdout);
}

fn nothing_under(home: &Path) {
    for dir in ["data", "run", "config"] {
        assert!(!home.join(dir).exists(), "{dir} was created under {home:?}");
    }
}

#[test]
fn a_start_with_the_nonce_descriptor_unset_exits_2() {
    let home = cortexkit_test_support::ScratchDir::new("occipital-nonce-unset");
    assert_nonce_refused(&run(ck_occipital(
        home.path(),
        &["--subc", "/nonexistent/connection"],
    )));
    nothing_under(home.path());
}

#[test]
fn a_start_naming_a_closed_descriptor_exits_2() {
    let home = cortexkit_test_support::ScratchDir::new("occipital-nonce-closed");
    let mut command = ck_occipital(home.path(), &["--subc", "/nonexistent/connection"]);
    command.env("SUBC_LAUNCH_NONCE_FD", CLOSED_DESCRIPTOR);
    assert_nonce_refused(&run(command));
    nothing_under(home.path());
}

#[test]
fn the_environment_copy_of_the_nonce_is_never_used() {
    let home = cortexkit_test_support::ScratchDir::new("occipital-nonce-env");
    let mut command = ck_occipital(home.path(), &["--subc", "/nonexistent/connection"]);
    command.env("SUBC_LAUNCH_NONCE", "a".repeat(64));
    assert_nonce_refused(&run(command));
}

#[test]
fn a_start_with_no_arguments_still_checks_the_nonce_first() {
    let home = cortexkit_test_support::ScratchDir::new("occipital-nonce-bare");
    assert_nonce_refused(&run(ck_occipital(home.path(), &[])));
}

#[test]
fn version_prints_the_crate_version_and_full_commit_without_starting() {
    let home = cortexkit_test_support::ScratchDir::new("occipital-version");
    for nonce_fd in [None, Some(CLOSED_DESCRIPTOR)] {
        let mut command = ck_occipital(home.path(), &["--version"]);
        if let Some(fd) = nonce_fd {
            command.env("SUBC_LAUNCH_NONCE_FD", fd);
        }
        let output = run(command);
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert!(output.stderr.is_empty(), "stderr: {}", stderr(&output));
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            stdout,
            format!(
                "ck-occipital {} {}\n",
                env!("CARGO_PKG_VERSION"),
                env!("OCCIPITAL_BUILD_GIT_SHA")
            )
        );
        let commit = stdout.split_whitespace().nth(2).unwrap();
        if commit != "unknown" {
            assert_eq!(commit.len(), 40, "full git commit: {stdout}");
            assert!(commit
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        } else {
            assert!(matches!(
                env!("OCCIPITAL_BUILD_GIT_TREE"),
                "no_git_dir" | "never_derived"
            ));
        }
        nothing_under(home.path());
    }
}

fn manifest_probe(nonce_fd: Option<&str>) -> Output {
    let home = cortexkit_test_support::ScratchDir::new("occipital-manifest");
    let mut command = ck_occipital(home.path(), &["--manifest"]);
    if let Some(fd) = nonce_fd {
        command.env("SUBC_LAUNCH_NONCE_FD", fd);
    }
    let output = run(command);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    nothing_under(home.path());
    output
}

#[test]
fn manifest_is_printed_before_the_nonce_is_read() {
    // With the nonce variable naming a closed descriptor, any start that read
    // the nonce would exit 2. `--manifest` must not, and must print the same
    // bytes it prints with no variable at all.
    let named = manifest_probe(Some(CLOSED_DESCRIPTOR));
    let unset = manifest_probe(None);
    assert!(named.stderr.is_empty(), "stderr: {}", stderr(&named));
    assert_eq!(named.stdout, unset.stdout);
    let manifest: Value = serde_json::from_slice(&named.stdout).expect("stdout is JSON");
    assert_eq!(manifest["module_id"], "occipital");
}

// The parse `place-module.sh` applies to `--manifest` output, ported from
// subconscious 7a6be091 (tests/fixtures/pinned/subconscious-7a6be091/
// place-module.sh.*.excerpt) so this test can apply it to the binary's real
// output. Python's `json.loads` with its `unique_object` hook refuses an
// object with a repeated key, compared after escapes are decoded.

/// A JSON syntax walk that reports the first object with a repeated key.
struct Keys<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Keys<'_> {
    fn ws(&mut self) {
        while self.at < self.bytes.len() && b" \t\n\r".contains(&self.bytes[self.at]) {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        self.ws();
        if self.bytes.get(self.at) == Some(&byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("expected {:?} at {}", byte as char, self.at))
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut units: Vec<u16> = Vec::new();
        loop {
            let rest = std::str::from_utf8(&self.bytes[self.at..]).map_err(|e| e.to_string())?;
            let c = rest.chars().next().ok_or("unterminated string")?;
            match c {
                '"' => {
                    self.at += 1;
                    return String::from_utf16(&units).map_err(|e| e.to_string());
                }
                '\\' => {
                    let escape = self.bytes.get(self.at + 1).copied().ok_or("bad escape")?;
                    self.at += 2;
                    let unit = match escape {
                        b'"' => u16::from(b'"'),
                        b'\\' => u16::from(b'\\'),
                        b'/' => u16::from(b'/'),
                        b'b' => 0x08,
                        b'f' => 0x0c,
                        b'n' => u16::from(b'\n'),
                        b'r' => u16::from(b'\r'),
                        b't' => u16::from(b'\t'),
                        b'u' => {
                            let hex = self.bytes.get(self.at..self.at + 4).ok_or("short \\u")?;
                            self.at += 4;
                            u16::from_str_radix(std::str::from_utf8(hex).unwrap(), 16)
                                .map_err(|e| e.to_string())?
                        }
                        other => return Err(format!("bad escape {:?}", other as char)),
                    };
                    units.push(unit);
                }
                c => {
                    let mut buffer = [0u16; 2];
                    units.extend_from_slice(c.encode_utf16(&mut buffer));
                    self.at += c.len_utf8();
                }
            }
        }
    }

    fn value(&mut self) -> Result<(), String> {
        self.ws();
        match self.bytes.get(self.at) {
            Some(b'{') => {
                self.at += 1;
                let mut seen = HashSet::new();
                self.ws();
                if self.bytes.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(());
                }
                loop {
                    let key = self.string()?;
                    if !seen.insert(key.clone()) {
                        return Err(format!("duplicate JSON object key {key}"));
                    }
                    self.expect(b':')?;
                    self.value()?;
                    self.ws();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(());
                        }
                        _ => return Err(format!("bad object at {}", self.at)),
                    }
                }
            }
            Some(b'[') => {
                self.at += 1;
                self.ws();
                if self.bytes.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(());
                }
                loop {
                    self.value()?;
                    self.ws();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            return Ok(());
                        }
                        _ => return Err(format!("bad array at {}", self.at)),
                    }
                }
            }
            Some(b'"') => self.string().map(|_| ()),
            Some(_) => {
                // Numbers and literals: their syntax is checked by the full
                // parse below; only object keys matter here.
                while self.at < self.bytes.len() && !b",}] \t\n\r".contains(&self.bytes[self.at]) {
                    self.at += 1;
                }
                Ok(())
            }
            None => Err("unexpected end".into()),
        }
    }
}

/// The face probe's answer for `--manifest` stdout (place-module.sh lines
/// 379-431): `not_checked`, `absent`, `present` or `invalid: <why>`.
fn face_probe(stdout: &[u8], binary: &str) -> String {
    let mut keys = Keys {
        bytes: stdout,
        at: 0,
    };
    if let Err(why) = keys.value() {
        if why.starts_with("duplicate JSON object key") {
            return format!("invalid: {why}");
        }
        return "not_checked".into();
    }
    let Ok(manifest) = serde_json::from_slice::<Value>(stdout) else {
        return "not_checked".into();
    };
    let Some(object) = manifest.as_object() else {
        return "not_checked".into();
    };
    let Some(faces) = object.get("cli_faces") else {
        return "absent".into();
    };
    let Some(faces) = faces.as_array() else {
        return "invalid: cli_faces must be an array".into();
    };
    let mut names = HashSet::new();
    for (index, face) in faces.iter().enumerate() {
        let Some(face) = face
            .as_object()
            .filter(|f| f.len() == 2 && f.contains_key("name") && f.contains_key("binary"))
        else {
            return format!("invalid: cli_faces[{index}] must contain exactly name and binary");
        };
        let name = face["name"].as_str().unwrap_or_default();
        let lowercase = !name.is_empty()
            && name.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            });
        if name.starts_with("ck-") || !lowercase {
            return format!("invalid: cli_faces[{index}].name is not a lowercase command name");
        }
        if !names.insert(name.to_owned()) {
            return format!("invalid: duplicate cli face name {name}");
        }
        if face["binary"].as_str() != Some(binary) {
            return format!("invalid: cli_faces[{index}].binary must equal {binary}");
        }
    }
    "present".into()
}

/// Python truthiness of a JSON value, as `x or default` reads it.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// The provenance line (place-module.sh lines 897-906): `stamped <sha>`,
/// `unstamped <why>` or `unreported`.
fn provenance_line(stdout: &[u8]) -> String {
    let Ok(manifest) = serde_json::from_slice::<Value>(stdout) else {
        return "unreported".into();
    };
    let Some(object) = manifest.as_object() else {
        return "unreported".into();
    };
    let empty = Value::Object(Default::default());
    let provenance = object
        .get("provenance")
        .filter(|p| truthy(p))
        .unwrap_or(&empty);
    let Some(provenance) = provenance.as_object() else {
        return "unreported".into();
    };
    let field = |name: &str| -> Result<String, ()> {
        match provenance.get(name) {
            Some(value) if truthy(value) => value.as_str().map(str::to_owned).ok_or(()),
            _ => Ok(String::new()),
        }
    };
    let (Ok(sha), Ok(why)) = (
        field("build_git_sha"),
        field("build_git_sha_absence_reason"),
    ) else {
        return "unreported".into();
    };
    if !sha.is_empty() {
        format!("stamped {sha}")
    } else if !why.is_empty() {
        format!("unstamped {why}")
    } else {
        "unreported".into()
    }
}

#[test]
fn the_ported_parse_refuses_a_repeated_key_and_reads_faces_and_provenance() {
    // The port must be able to fail, or the manifest test below proves nothing.
    assert_eq!(
        face_probe(br#"{"a":1,"\u0061":2}"#, "ck-occipital"),
        "invalid: duplicate JSON object key a"
    );
    assert_eq!(face_probe(b"[1]", "ck-occipital"), "not_checked");
    assert_eq!(face_probe(b"not json", "ck-occipital"), "not_checked");
    assert_eq!(
        face_probe(
            br#"{"cli_faces":[{"name":"ui","binary":"ck-occipital"}]}"#,
            "ck-occipital"
        ),
        "present"
    );
    assert_eq!(
        face_probe(
            br#"{"cli_faces":[{"name":"ck-ui","binary":"ck-occipital"}]}"#,
            "ck-occipital"
        ),
        "invalid: cli_faces[0].name is not a lowercase command name"
    );
    assert_eq!(
        provenance_line(br#"{"provenance":{"build_git_sha":"abc"}}"#),
        "stamped abc"
    );
    assert_eq!(
        provenance_line(br#"{"provenance":{"build_git_sha_absence_reason":"declined_dirty"}}"#),
        "unstamped declined_dirty"
    );
    assert_eq!(provenance_line(br#"{"provenance":null}"#), "unreported");
    assert_eq!(
        provenance_line(br#"{"provenance":{"build_git_sha":5}}"#),
        "unreported"
    );
}

#[test]
fn manifest_output_matches_the_place_module_parse_fixture() {
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/manifest/place-module-parse.json"),
        )
        .expect("read the parse fixture"),
    )
    .expect("the parse fixture is JSON");
    let stdout = manifest_probe(None).stdout;

    assert_eq!(
        face_probe(&stdout, "ck-occipital"),
        fixture["face_probe_result"].as_str().unwrap()
    );

    let tree = env!("OCCIPITAL_BUILD_GIT_TREE");
    let expected = fixture["provenance_line"][tree]
        .as_str()
        .unwrap_or_else(|| panic!("the fixture has no line for tree state {tree}"))
        .replace("{build_git_sha}", env!("OCCIPITAL_BUILD_GIT_SHA"));
    assert_eq!(provenance_line(&stdout), expected);

    // One line of JSON, as the script reads it from a pipe.
    let text = std::str::from_utf8(&stdout).unwrap();
    assert!(text.ends_with('\n'));
    assert_eq!(text.lines().count(), 1);
}
