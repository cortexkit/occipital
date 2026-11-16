#!/usr/bin/env bash
# Checks that the committed ports/swift/OccipitalCore.xcframework was built
# from the current sources: recomputes scripts/xcframework-source-hash.sh and
# compares it with the hash scripts/build-xcframework.sh recorded in
# ports/swift/OccipitalCore.source-hash. It does not inspect the archives'
# contents. It also checks the layout: the two expected slices are present,
# each archive is nonempty, and each copied header matches the crate's
# occipital.h. A stale or broken library is rebuilt with
# scripts/build-xcframework.sh on the build server.
#
# It also checks that every hand-written source file stays under 5,000 lines,
# so a one-line edit rebuilds quickly and no file grows into one that every
# feature passes through. Tests and generated files are exempt.
#
# Exits 0 when everything matches. Otherwise it names each problem and exits 1.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

xcframework="ports/swift/OccipitalCore.xcframework"
hash_file="ports/swift/OccipitalCore.source-hash"
header="crates/occipital-ffi/include/occipital.h"
library="liboccipital_ffi.a"

problems=0
problem() {
  echo "check: $*" >&2
  problems=$((problems + 1))
}

expected_modulemap='module OccipitalCore {
    header "occipital.h"
    export *
}'

[ -f "$xcframework/Info.plist" ] || problem "$xcframework/Info.plist is missing"
for identifier in ios-arm64 ios-arm64-simulator; do
  slice="$xcframework/$identifier"
  [ -s "$slice/$library" ] || problem "$slice/$library is missing or empty"
  if ! cmp -s "$header" "$slice/Headers/OccipitalCore/occipital.h"; then
    problem "$slice/Headers/OccipitalCore/occipital.h differs from $header"
  fi
  if [ "$(cat "$slice/Headers/OccipitalCore/module.modulemap" 2>/dev/null)" != "$expected_modulemap" ]; then
    problem "$slice/Headers/OccipitalCore/module.modulemap is missing or does not declare module OccipitalCore"
  fi
  # A module map directly under Headers would clash with other static
  # xcframeworks' in the app's shared include folder.
  if [ -e "$slice/Headers/module.modulemap" ] || [ -e "$slice/Headers/occipital.h" ]; then
    problem "$slice/Headers must hold only the OccipitalCore folder"
  fi
done
for entry in "$xcframework"/*; do
  [ -e "$entry" ] || continue
  case "${entry##*/}" in
    Info.plist | ios-arm64 | ios-arm64-simulator) ;;
    *) problem "unexpected entry $entry (only ios-arm64 and ios-arm64-simulator ship)" ;;
  esac
done

if [ ! -f "$hash_file" ]; then
  problem "$hash_file is missing"
else
  recorded="$(cat "$hash_file")"
  if ! current="$(scripts/xcframework-source-hash.sh)"; then
    problem "scripts/xcframework-source-hash.sh failed"
  elif [ "$recorded" != "$current" ]; then
    problem "$xcframework is stale: $hash_file records $recorded, but the sources hash to $current"
  fi
fi

max_lines=5000
while IFS= read -r file; do
  case "$file" in
    */tests/* | */tests.rs | *_tests.rs | */test-vectors/* | */vectors/*) continue ;;
  esac
  # A generated file says so on its first line (the Unicode tables do).
  head -n 1 "$file" | grep -q 'Generated' && continue
  lines="$(wc -l <"$file")"
  if [ "$lines" -ge "$max_lines" ]; then
    problem "$file has $((lines)) lines; split it by area (hand-written source stays under $max_lines)"
  fi
done < <(git ls-files '*.rs' '*.swift' '*.py' '*.sh')

if [ "$problems" -gt 0 ]; then
  echo "check: $problems problem(s)" >&2
  exit 1
fi
echo "check: $xcframework matches its sources ($current); every source file is under $max_lines lines"
