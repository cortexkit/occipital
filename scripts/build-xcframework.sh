#!/usr/bin/env bash
# Builds ports/swift/OccipitalCore.xcframework, the static library apps link to
# embed occipital-lang, and records its source hash in
# ports/swift/OccipitalCore.source-hash.
#
# Runs on the Linux build server with its shared Rust 1.99.0 toolchain, which
# must carry the aarch64-apple-ios and aarch64-apple-ios-sim targets and the
# llvm-tools component. The script never installs a toolchain, target or
# component: if one is missing it stops and names it.
#
# Every dependency of occipital-ffi is pure Rust, so building a static library
# for iOS needs only the targets' standard libraries, no Apple linker or SDK.
# Each archive is built with fat LTO, one codegen unit, panic=unwind (the FFI
# turns panics into OCC_PANIC) and IPHONEOS_DEPLOYMENT_TARGET=17.0, without the
# `test-hooks` feature of occipital-ffi or the `traces` feature of
# occipital-lang. It is then stripped of debug info with `llvm-strip -S`.
#
# The archives are reproducible: two clean builds of the same sources on the
# build server give byte-identical files. rustc writes archive members with
# zero timestamps and owners, the checkout and registry paths are remapped
# below, and `llvm-strip -S` is deterministic.
#
# The xcframework has two slices, ios-arm64 and ios-arm64-simulator, each with
# Headers/OccipitalCore/occipital.h and Headers/OccipitalCore/module.modulemap.
# The headers sit in a folder named after the module because Xcode copies
# every static xcframework's Headers into one shared include folder: an app
# that also links another such xcframework (AvatarKit's AvatarCore) would
# otherwise get two include/module.modulemap files and refuse to build. The
# script writes its Info.plist itself, deterministically, with keys in sorted
# order.
#
# Usage:
#   scripts/build-xcframework.sh                 build and write the xcframework
#   scripts/build-xcframework.sh --archives-only build the stripped archives and
#                                                staged headers under
#                                                target/xcframework/ and stop
#
# Mac fallback for writing the xcframework, if the hand-written Info.plist ever
# stops being accepted by Xcode: run `--archives-only` on Linux, bring
# target/xcframework/ to the Mac, then from the repository root run
#
#   rm -rf ports/swift/OccipitalCore.xcframework
#   xcodebuild -create-xcframework \
#     -library target/xcframework/ios-arm64/liboccipital_ffi.a \
#     -headers target/xcframework/headers \
#     -library target/xcframework/ios-arm64-simulator/liboccipital_ffi.a \
#     -headers target/xcframework/headers \
#     -output ports/swift/OccipitalCore.xcframework
#   cp target/xcframework/OccipitalCore.source-hash ports/swift/OccipitalCore.source-hash
#
# which produces the same layout.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

archives_only=0
case "${1:-}" in
  "") ;;
  --archives-only) archives_only=1 ;;
  *)
    echo "usage: scripts/build-xcframework.sh [--archives-only]" >&2
    exit 2
    ;;
esac

fail() {
  echo "build-xcframework: $*" >&2
  exit 1
}

required_rust="1.99.0"
xcframework="ports/swift/OccipitalCore.xcframework"
hash_file="ports/swift/OccipitalCore.source-hash"
header="crates/occipital-ffi/include/occipital.h"
library="liboccipital_ffi.a"
stage="target/xcframework"

# Rust target triple and xcframework library identifier of each slice.
slices=(
  "aarch64-apple-ios ios-arm64"
  "aarch64-apple-ios-sim ios-arm64-simulator"
)

# --- Toolchain -------------------------------------------------------------

command -v rustc >/dev/null 2>&1 || fail "rustc is not on PATH"
command -v cargo >/dev/null 2>&1 || fail "cargo is not on PATH"

rustc_version="$(rustc --version)"
case "$rustc_version" in
  "rustc $required_rust "*) ;;
  *) fail "needs the shared Rust $required_rust toolchain, found: $rustc_version" ;;
esac

sysroot="$(rustc --print sysroot)"
host="$(rustc -vV | sed -n 's/^host: //p')"
tools="$sysroot/lib/rustlib/$host/bin"

missing=()
for slice in "${slices[@]}"; do
  target="${slice%% *}"
  if [ ! -d "$sysroot/lib/rustlib/$target/lib" ]; then
    missing+=("target $target (rust-std-$target)")
  fi
done
for tool in llvm-strip llvm-nm; do
  if [ ! -x "$tools/$tool" ]; then
    missing+=("component llvm-tools ($tools/$tool)")
    break
  fi
done
if [ "${#missing[@]}" -gt 0 ]; then
  {
    echo "build-xcframework: the Rust toolchain at $sysroot ($rustc_version) is missing:"
    for item in "${missing[@]}"; do
      echo "  - $item"
    done
    echo "This script does not install toolchain pieces; the build server's owner must add them."
  } >&2
  exit 1
fi

# --- Source hash -----------------------------------------------------------

source_hash="$(scripts/xcframework-source-hash.sh)"

# --- Archives --------------------------------------------------------------

# Release settings for the shipped archive, set here rather than in the
# workspace profile so other release builds keep their own settings.
export CARGO_PROFILE_RELEASE_LTO=fat
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
export CARGO_PROFILE_RELEASE_PANIC=unwind
export IPHONEOS_DEPLOYMENT_TARGET=17.0

# Keep the build machine's checkout and registry paths out of the panic
# location strings compiled into the archive.
rustflags="--remap-path-prefix=$root=/occipital"
cargo_home="${CARGO_HOME:-${HOME:+$HOME/.cargo}}"
if [ -n "$cargo_home" ]; then
  rustflags="$rustflags --remap-path-prefix=$cargo_home=/cargo"
fi
export RUSTFLAGS="$rustflags"

rm -rf "$stage"
mkdir -p "$stage/headers"
cp "$header" "$stage/headers/occipital.h"
cat >"$stage/headers/module.modulemap" <<'EOF'
module OccipitalCore {
    header "occipital.h"
    export *
}
EOF

for slice in "${slices[@]}"; do
  target="${slice%% *}"
  identifier="${slice##* }"

  # `cargo rustc --crate-type staticlib` rather than `cargo build`: the crate
  # also declares an rlib, and cargo silently skips LTO for a mixed
  # staticlib+rlib build, which would ship every dependency's objects whole.
  cargo rustc --locked --release --lib \
    --package occipital-ffi --no-default-features --target "$target" \
    --crate-type staticlib

  built="target/$target/release/$library"
  [ -f "$built" ] || fail "cargo did not produce $built"

  mkdir -p "$stage/$identifier"
  # Not `-x`: on Mach-O objects this toolchain's llvm-strip deletes local
  # symbols that relocations still use, which corrupts the relocations and
  # makes the output differ from run to run. The app's own build strips
  # local symbols from its final binary.
  "$tools/llvm-strip" -S -o "$stage/$identifier/$library" "$built"

  # The shipped archive must export the C entry points and must not carry the
  # test-only hooks.
  symbols="$("$tools/llvm-nm" --extern-only --defined-only "$stage/$identifier/$library" 2>/dev/null)"
  grep -q ' _occ_version$' <<<"$symbols" ||
    fail "$identifier archive does not define _occ_version"
  if grep -q ' _occ_test_' <<<"$symbols"; then
    fail "$identifier archive carries test-only entry points"
  fi

  before="$(wc -c <"$built" | tr -d ' ')"
  after="$(wc -c <"$stage/$identifier/$library" | tr -d ' ')"
  echo "$identifier ($target): $before bytes before stripping, $after bytes after"
done

if [ "$(scripts/xcframework-source-hash.sh)" != "$source_hash" ]; then
  fail "a hashed input changed during the build; run the script again"
fi
printf '%s\n' "$source_hash" >"$stage/OccipitalCore.source-hash"

if [ "$archives_only" -eq 1 ]; then
  echo "archives and headers are in $stage; see this script's header for the Mac step"
  exit 0
fi

# --- xcframework -----------------------------------------------------------

next="$xcframework.next"
rm -rf "$next"
mkdir -p "$next"
for slice in "${slices[@]}"; do
  identifier="${slice##* }"
  mkdir -p "$next/$identifier/Headers/OccipitalCore"
  cp "$stage/$identifier/$library" "$next/$identifier/$library"
  cp "$stage/headers/occipital.h" "$stage/headers/module.modulemap" \
    "$next/$identifier/Headers/OccipitalCore/"
done

# The layout `xcodebuild -create-xcframework` writes for these two libraries,
# with every dictionary's keys in sorted order.
cat >"$next/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AvailableLibraries</key>
	<array>
		<dict>
			<key>BinaryPath</key>
			<string>$library</string>
			<key>HeadersPath</key>
			<string>Headers</string>
			<key>LibraryIdentifier</key>
			<string>ios-arm64</string>
			<key>LibraryPath</key>
			<string>$library</string>
			<key>SupportedArchitectures</key>
			<array>
				<string>arm64</string>
			</array>
			<key>SupportedPlatform</key>
			<string>ios</string>
		</dict>
		<dict>
			<key>BinaryPath</key>
			<string>$library</string>
			<key>HeadersPath</key>
			<string>Headers</string>
			<key>LibraryIdentifier</key>
			<string>ios-arm64-simulator</string>
			<key>LibraryPath</key>
			<string>$library</string>
			<key>SupportedArchitectures</key>
			<array>
				<string>arm64</string>
			</array>
			<key>SupportedPlatform</key>
			<string>ios</string>
			<key>SupportedPlatformVariant</key>
			<string>simulator</string>
		</dict>
	</array>
	<key>CFBundlePackageType</key>
	<string>XFWK</string>
	<key>XCFrameworkFormatVersion</key>
	<string>1.0</string>
</dict>
</plist>
EOF

rm -rf "$xcframework"
mv "$next" "$xcframework"
cp "$stage/OccipitalCore.source-hash" "$hash_file"
echo "wrote $xcframework and $hash_file ($source_hash)"
