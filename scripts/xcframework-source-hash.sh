#!/usr/bin/env bash
# Prints the SHA-256 that identifies the sources ports/swift/OccipitalCore.xcframework
# was built from. scripts/build-xcframework.sh records it in
# ports/swift/OccipitalCore.source-hash, and scripts/check.sh recomputes it to
# refuse a committed binary that no longer matches its sources.
#
# Inputs, in byte-wise sorted path order:
#   - every git-tracked file under crates/occipital-lang/, crates/occipital-refs/
#     and crates/occipital-ffi/,
#     except crates/occipital-ffi/vectors/ and crates/occipital-refs/vectors/ (test
#     data, not compiled into the archive);
#   - the root Cargo.toml (it holds the release profile);
#   - scripts/build-xcframework.sh;
# followed by the locked versions and features of every package occipital-ffi
# builds with, and the output of `rustc --version`.
#
# The package list comes from `cargo tree --locked --offline` for occipital-ffi
# alone, not from the whole Cargo.lock, so a dependency change in another
# crate of the workspace (the module, say) does not mark the archives stale.
# `--target all` lists every platform's dependencies, so the list is the same
# on whichever computer runs this. Every parenthesized note cargo adds is
# dropped: a path package's location differs between checkouts, and the
# `(proc-macro)` and repeated-subtree `(*)` marks only restate the graph.
#
# File contents are read from the working tree, so an uncommitted edit to any
# input changes the hash. The digest is taken over a manifest with one
# "<sha256 of contents>  <path>" line per file, one "package: <name> v<version>
# <features>" line per package and a final "rustc: <version>" line, so it is
# the same on Linux (sha256sum) and macOS (shasum).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 | cut -d' ' -f1; }
else
  echo "xcframework-source-hash: neither sha256sum nor shasum is available" >&2
  exit 1
fi

rustc_version="$(rustc --version)"

# Taken before the manifest, so a failed resolution stops the script.
packages="$(cargo tree --locked --offline --quiet -p occipital-ffi --target all \
  -e normal,build --prefix none --format '{p} {f}' |
  sed -E 's/ \([^)]*\)//g; s/ +$//' | LC_ALL=C sort -u)"
if [ -z "$packages" ]; then
  echo "xcframework-source-hash: cargo tree listed no packages for occipital-ffi" >&2
  exit 1
fi

# Built in full before hashing, so a missing input fails the script instead of
# printing a digest of a partial manifest.
manifest="$({
  git ls-files -z -- crates/occipital-lang crates/occipital-refs crates/occipital-ffi \
    ':(exclude)crates/occipital-ffi/vectors' ':(exclude)crates/occipital-refs/vectors'
  printf '%s\0' Cargo.toml scripts/build-xcframework.sh
} | LC_ALL=C sort -z -u | {
  while IFS= read -r -d '' path; do
    if [ ! -f "$path" ]; then
      echo "xcframework-source-hash: hashed input $path is missing" >&2
      exit 1
    fi
    printf '%s  %s\n' "$(sha256 <"$path")" "$path"
  done
  printf '%s\n' "$packages" | sed 's/^/package: /'
  printf 'rustc: %s\n' "$rustc_version"
})"

printf '%s\n' "$manifest" | sha256
