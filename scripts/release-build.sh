#!/usr/bin/env bash
# Builds the release ck-occipital with its build commit stamped in, and checks
# that the binary reports that commit. The release lane runs it on each target's
# own runner; it also works on a development checkout. A package version alone
# can't tell builds from different commits apart, so the binary must name the
# exact clean commit it was built from.
set -euo pipefail

cd "$(dirname "$0")/.."

die() { printf 'release-build.sh: %s\n' "$*" >&2; exit 1; }

rev="$(git rev-parse HEAD)"
[[ "$rev" == "${RELEASE_REVISION:-$rev}" ]] || die "expected ${RELEASE_REVISION}, checked out $rev"

# A dirty tree's bytes would not be the commit the binary claims. Untracked
# files count too, since a release runner's checkout has none.
[[ -z "$(git status --porcelain --untracked-files=all)" ]] ||
  die "refusing to build a dirty tree as $rev: commit first, or use a plain cargo build"

echo "building ck-occipital at $rev"
CARGO_TARGET_DIR=target cargo build --locked --release -p occipital-module --bin ck-occipital

[[ "$(git rev-parse HEAD)" == "$rev" && -z "$(git status --porcelain --untracked-files=all)" ]] ||
  die "the checkout changed during the build (expected clean $rev)"

suffix=""
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) suffix=.exe ;;
esac
built="target/release/ck-occipital$suffix"

# The `ck-` names are kept for installed programs, so the probe runs the same
# bytes under a `ckdev-` name.
probe_dir="$(mktemp -d)"
trap 'rm -rf "$probe_dir"' EXIT
probe="$probe_dir/ckdev-occipital$suffix"
cp "$built" "$probe"

version="$("$probe" --version)" || die "the --version probe failed"
[[ "$version" =~ ^ck-occipital\ [^\ ]+\ ([0-9a-f]{40})$ && "${BASH_REMATCH[1]}" == "$rev" ]] ||
  die "ck-occipital does not report revision $rev: ${version:-<no output>}"
echo "$version"

# A Mach-O debug map names object files on the build machine. Cargo can finish
# even when stripping failed, so look for it.
if [[ "$(uname -s)" == Darwin ]]; then
  entries="$(nm -a "$built" | grep -c ' OSO ' || true)"
  [[ "$entries" == 0 ]] || die "ck-occipital still carries $entries debug-map entries"
  echo "debug map: 0 entries"
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$built"
else
  shasum -a 256 "$built"
fi
