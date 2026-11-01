//! Embeds the commit this binary is built from, so `ck-occipital --manifest` can
//! declare its build provenance. The fleet's placement script reads that
//! declaration to refuse a binary built from a commit other than the one its
//! owner named, or from a tree with uncommitted changes.
//!
//! Two variables reach the crate:
//! - `OCCIPITAL_BUILD_GIT_SHA`: the 40-character commit, or `unknown`.
//! - `OCCIPITAL_BUILD_GIT_TREE`: `clean` or `dirty` when a commit was found,
//!   `no_git_dir` when the source is not inside a git checkout, and
//!   `never_derived` when git was there but gave no usable commit.
//!
//! A build script failure would stop every build, so any git problem becomes
//! one of the honest "no commit" answers instead.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|text| text.trim_end_matches(['\n', '\r']).to_owned())
}

fn is_commit(text: &str) -> bool {
    text.len() == 40
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Ask Cargo to rerun this script when the commit or the staged state changes.
/// The working tree's files are watched through the crates directory and the
/// root manifests, so editing source also refreshes the dirty flag.
fn watch(dir: &Path) {
    for git_path in ["HEAD", "index", "packed-refs"] {
        if let Some(path) = git(dir, &["rev-parse", "--git-path", git_path]) {
            println!("cargo:rerun-if-changed={}", dir.join(path).display());
        }
    }
    if let Some(reference) = git(dir, &["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(dir, &["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={}", dir.join(path).display());
        }
    }
    let workspace = dir.join("..").join("..");
    for path in ["crates", "Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={}", workspace.join(path).display());
    }
}

fn main() {
    let dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    watch(&dir);

    let (sha, tree) = match git(&dir, &["rev-parse", "--is-inside-work-tree"]) {
        None => ("unknown".to_owned(), "no_git_dir"),
        Some(_) => match git(&dir, &["rev-parse", "HEAD"]).filter(|sha| is_commit(sha)) {
            None => ("unknown".to_owned(), "never_derived"),
            Some(sha) => {
                // Untracked files are left out, so scratch files beside a
                // checkout do not mark every build dirty.
                let tree = match git(&dir, &["status", "--porcelain", "--untracked-files=no"]) {
                    Some(status) if status.is_empty() => "clean",
                    _ => "dirty",
                };
                (sha, tree)
            }
        },
    };
    println!("cargo:rustc-env=OCCIPITAL_BUILD_GIT_SHA={sha}");
    println!("cargo:rustc-env=OCCIPITAL_BUILD_GIT_TREE={tree}");
}
