//! How the `ck-occipital` process starts, in the order the daemon relies on.
//!
//! ```text
//! ck-occipital --manifest      print the manifest (with its build provenance) and exit 0
//! ck-occipital --version       print the crate version and build commit and exit 0
//! ck-occipital --subc <file>    serve (how the daemon starts it)
//! ```
//!
//! `--manifest` and `--version` are handled before anything else: neither reads
//! the launch nonce, opens the store or connects to the daemon, so the placement script
//! can probe a staged binary safely. Every other start reads the launch nonce
//! first, before the process spawns or opens anything, and refuses to go on
//! without one.

use std::process::ExitCode;

use subc_os::{LaunchNonce, LaunchNonceError, LaunchNonceSource};

/// The exit status when the launch nonce is missing or unreadable.
pub const EXIT_NONCE_UNAVAILABLE: u8 = 2;

/// The exit status when the store cannot be served, including a store written
/// by a newer build.
pub const EXIT_STORE_FAILURE: u8 = 1;

/// The exit status for every other failure to start or keep serving.
pub const EXIT_SERVE_FAILURE: u8 = 1;

/// The start of the stderr line written when the launch nonce is unavailable.
pub const NONCE_UNAVAILABLE_PREFIX: &str = "ck-occipital: launch nonce unavailable:";

/// A reason the process must stop, with the one stderr line it writes and
/// the status it exits with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fatal {
    pub code: u8,
    pub line: String,
}

impl Fatal {
    pub fn new(code: u8, prefix: &str, reason: impl std::fmt::Display) -> Self {
        Self {
            code,
            line: format!("{prefix} {reason}"),
        }
    }

    /// Write the line and turn the status into an exit code.
    pub fn report(&self) -> ExitCode {
        eprintln!("{}", self.line);
        ExitCode::from(self.code)
    }
}

/// Decide whether the nonce this process read may be used.
///
/// The daemon hands a module its nonce through a descriptor (`fd`, named by
/// `SUBC_LAUNCH_NONCE_FD`) or, on Windows, a one-time named pipe (`pipe`).
/// `subc_os::launch_nonce` still falls back to the `SUBC_LAUNCH_NONCE`
/// environment copy when neither is named, but any process of the same user
/// can read another's environment, so Occipital never accepts a nonce from
/// there. A missing nonce has no default either: without one the daemon would
/// not admit this process as the reserved module.
pub fn require_launch_nonce(
    read: Result<Option<LaunchNonce>, LaunchNonceError>,
) -> Result<LaunchNonce, Fatal> {
    let refuse =
        |reason: String| Fatal::new(EXIT_NONCE_UNAVAILABLE, NONCE_UNAVAILABLE_PREFIX, reason);
    match read {
        Ok(Some(nonce)) => match nonce.source() {
            LaunchNonceSource::Fd | LaunchNonceSource::Pipe => Ok(nonce),
            other => Err(refuse(format!(
                "the nonce came from source {:?}, not the daemon's descriptor or pipe; \
                 ck-occipital never reads it from the environment",
                other.as_str()
            ))),
        },
        Ok(None) => Err(refuse(format!(
            "{} is not set (nor, on Windows, {}); ck-occipital must be started by the daemon",
            subc_os::LAUNCH_NONCE_FD_ENV,
            subc_os::LAUNCH_NONCE_PIPE_ENV,
        ))),
        Err(error) => Err(refuse(error.to_string())),
    }
}

/// What the command line asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Print the manifest and exit.
    Manifest,
    /// Print the crate version and embedded build commit and exit.
    Version,
    /// Read the nonce and serve. The SDK reads `--subc <file>` itself.
    Serve,
}

/// Only `--manifest` or `--version` on its own is a safe probe. Anything else is a
/// start that must prove its launch nonce before the arguments matter, so a
/// mistyped command can never skip the nonce check.
pub fn mode(args: &[String]) -> Mode {
    match args {
        [flag] if flag == "--manifest" => Mode::Manifest,
        [flag] if flag == "--version" => Mode::Version,
        _ => Mode::Serve,
    }
}

/// Print the manifest for `--manifest`.
pub fn print_manifest() -> ExitCode {
    match crate::manifest::manifest_json() {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => Fatal::new(EXIT_SERVE_FAILURE, "ck-occipital:", error).report(),
    }
}

/// The whole process: safe probes first, then the nonce, then serving.
pub fn run(args: &[String]) -> ExitCode {
    match mode(args) {
        Mode::Manifest => return print_manifest(),
        Mode::Version => {
            println!(
                "ck-occipital {} {}",
                env!("CARGO_PKG_VERSION"),
                crate::manifest::BUILD_GIT_SHA
            );
            return ExitCode::SUCCESS;
        }
        Mode::Serve => {}
    }
    let nonce = match require_launch_nonce(subc_os::launch_nonce()) {
        Ok(nonce) => nonce,
        Err(fatal) => return fatal.report(),
    };
    // The fleet log, installed before anything that logs. A start the daemon
    // did not make (no `SUBC_MODULE_ID`) serves without it rather than refusing.
    if let Err(error) = cortexkit_log::init_from_env() {
        eprintln!("ck-occipital: fleet log unavailable: {error}");
    }
    let Some(connection_file) = connection_file(args) else {
        return Fatal::new(
            EXIT_SERVE_FAILURE,
            "ck-occipital:",
            "missing required --subc <connection-file> argument",
        )
        .report();
    };
    // The app reader calls the runners and Entorhinal as this module, proved by
    // the same nonce, so they see `reserved:occipital` rather than a plain caller.
    let consumer = crate::serve::Consumer {
        connection_file,
        identity: subc_client_rs::ConsumerIdentity {
            module_id: crate::MODULE_ID.into(),
            launch_nonce: nonce.value().into(),
        },
    };
    match crate::serve::run(consumer) {
        Ok(()) => ExitCode::SUCCESS,
        Err(fatal) => fatal.report(),
    }
}

/// The daemon's connection file, from `--subc <file>` or `--subc=<file>`, as
/// the SDK reads it to serve.
pub fn connection_file(args: &[String]) -> Option<std::path::PathBuf> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--subc" {
            return args.next().filter(|path| !path.is_empty()).map(Into::into);
        }
        if let Some(path) = arg.strip_prefix("--subc=").filter(|path| !path.is_empty()) {
            return Some(path.into());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_connection_file_is_read_as_the_sdk_reads_it() {
        assert_eq!(
            connection_file(&args(&["--subc", "/run/c.json"])),
            Some("/run/c.json".into())
        );
        assert_eq!(
            connection_file(&args(&["x", "--subc=/run/c.json"])),
            Some("/run/c.json".into())
        );
        for missing in [
            &[][..],
            &["--subc"][..],
            &["--subc="][..],
            &["--subc", ""][..],
        ] {
            assert_eq!(connection_file(&args(missing)), None, "{missing:?}");
        }
    }

    #[test]
    fn only_lone_probe_flags_skip_the_nonce() {
        assert_eq!(mode(&args(&["--manifest"])), Mode::Manifest);
        assert_eq!(mode(&args(&["--version"])), Mode::Version);
        for other in [
            &[][..],
            &["--subc", "/x"][..],
            &["--manifest", "--subc", "/x"][..],
            &["--subc", "/x", "--manifest"][..],
            &["--manifest="][..],
            &["--version", "--subc", "/x"][..],
            &["--subc", "/x", "--version"][..],
            &["--version="][..],
        ] {
            assert_eq!(mode(&args(other)), Mode::Serve, "{other:?}");
        }
    }

    fn pinned(path: &str) -> String {
        let full = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pinned")
            .join(path);
        std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{}: {e}", full.display()))
    }

    /// The error variants the pinned subc-os excerpt declares.
    fn pinned_error_variants() -> Vec<String> {
        pinned("subc-os-0.1.19/launch_nonce.rs.L99-L138.excerpt")
            .lines()
            .filter_map(|line| {
                let name = line.strip_prefix("    ")?;
                let end = name.find([',', ' ', '{'])?;
                let name = &name[..end];
                name.starts_with(|c: char| c.is_ascii_uppercase())
                    .then(|| name.to_owned())
            })
            .collect()
    }

    #[test]
    fn the_gate_is_typed_against_the_pinned_launch_nonce_signature() {
        let excerpt = pinned("subc-os-0.1.19/launch_nonce.rs.L223-L280.excerpt");
        assert!(excerpt
            .contains("pub fn launch_nonce() -> Result<Option<LaunchNonce>, LaunchNonceError> {"));
        // The accessor falls back to the environment copy when no descriptor
        // is named, which is why the gate checks the nonce's source.
        assert!(excerpt.contains("Ok(environment_nonce(lookup(LAUNCH_NONCE_ENV)))"));
        let _: fn() -> Result<Option<LaunchNonce>, LaunchNonceError> = subc_os::launch_nonce;
        let contract = pinned("subconscious-7a6be091/launch-nonce.md.L66-L99.excerpt");
        assert!(contract.contains(
            "`subc_os::launch_nonce() -> Result<Option<LaunchNonce>, LaunchNonceError>`"
        ));
    }

    #[test]
    fn the_handoff_error_test_covers_every_pinned_variant() {
        let variants = pinned_error_variants();
        assert_eq!(
            variants,
            [
                "Malformed",
                "NotOpen",
                "NotAPipe",
                "WrongPipe",
                "Empty",
                "Unreadable",
                "NotUtf8",
                "PipeNotOpen",
                "PipeEmpty",
                "PipeUnreadable",
                "PipeNotUtf8",
            ]
        );
    }

    #[test]
    fn no_named_handoff_is_refused_with_exit_2() {
        let fatal = require_launch_nonce(Ok(None)).unwrap_err();
        assert_eq!(fatal.code, 2);
        assert!(
            fatal.line.starts_with(NONCE_UNAVAILABLE_PREFIX),
            "{}",
            fatal.line
        );
        assert!(
            fatal.line.contains("SUBC_LAUNCH_NONCE_FD"),
            "{}",
            fatal.line
        );
    }

    #[test]
    fn every_named_handoff_error_is_refused_with_exit_2() {
        for error in [
            LaunchNonceError::Malformed { value: "3".into() },
            LaunchNonceError::NotOpen { fd: 3, errno: 9 },
            LaunchNonceError::NotAPipe { fd: 3 },
            LaunchNonceError::WrongPipe {
                fd: 3,
                expected_inode: 1,
                found_inode: 2,
            },
            LaunchNonceError::Empty { fd: 3 },
            LaunchNonceError::Unreadable { fd: 3, errno: None },
            LaunchNonceError::NotUtf8 { fd: 3 },
            LaunchNonceError::PipeNotOpen { errno: 2 },
            LaunchNonceError::PipeEmpty,
            LaunchNonceError::PipeUnreadable { errno: None },
            LaunchNonceError::PipeNotUtf8,
        ] {
            let expected = format!("{NONCE_UNAVAILABLE_PREFIX} {error}");
            let fatal = require_launch_nonce(Err(error)).unwrap_err();
            assert_eq!(fatal.code, EXIT_NONCE_UNAVAILABLE);
            assert_eq!(fatal.line, expected);
        }
    }
}
