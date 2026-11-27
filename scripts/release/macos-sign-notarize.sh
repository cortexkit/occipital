#!/usr/bin/env bash
# Signs ck-occipital with the Developer ID identity, packs it and notarizes the
# archive. It signs before packing, since the release lane checksums only the
# final ZIP.
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
source_dir="${1:?usage: $0 <source-dir> <asset-dir> <darwin-target>}"
asset_dir="${2:?asset directory is required}"
target="${3:?darwin target is required}"
[[ "$target" == darwin-arm64 ]] || { echo "not a darwin target: $target" >&2; exit 1; }

: "${APP_STORE_CONNECT_API_KEY_PATH:?App Store Connect API key path is required}"
: "${APP_STORE_CONNECT_API_KEY_ID:?App Store Connect API key ID is required}"
: "${APP_STORE_CONNECT_API_ISSUER_ID:?App Store Connect API issuer ID is required}"
[[ -r "$APP_STORE_CONNECT_API_KEY_PATH" ]] || { echo "App Store Connect API key is not readable" >&2; exit 1; }

# An Apple Development identity can sign but can't notarize, so require
# exactly one Developer ID Application identity before signing.
identity=$(security find-identity -v -p codesigning | awk -F'"' '$2 ~ /^Developer ID Application:/ { print $2 }')
if [[ -z "$identity" || "$identity" == *$'\n'* ]]; then
  echo "release requires exactly one valid Developer ID Application signing identity" >&2
  exit 1
fi

binary="$source_dir/ck-occipital"
# The identifier is fixed, since macOS privacy grants are bound to it. The
# hardened runtime refuses a same-user debugger. Without it, any program of the
# same user could attach and read the launch nonce, the one-time secret the
# daemon hands the module to prove its identity, and then call other modules
# as Occipital.
codesign --force --options runtime --timestamp --sign "$identity" --identifier ck-occipital "$binary"
codesign --verify --strict "$binary"
flags=$(codesign -dv "$binary" 2>&1 | sed -n 's/^CodeDirectory .*flags=\([^ ]*\).*/\1/p')
[[ "$flags" == *runtime* ]] || { echo "ck-occipital lacks the hardened runtime: $flags" >&2; exit 1; }
[[ "$(codesign -dv "$binary" 2>&1 | sed -n 's/^Identifier=//p')" == ck-occipital ]] ||
  { echo "ck-occipital is signed under the wrong identifier" >&2; exit 1; }
if codesign -d --entitlements - "$binary" 2>/dev/null | grep -q '<key>'; then
  echo "ck-occipital must carry no entitlements" >&2
  exit 1
fi

python "$script_dir/assets.py" pack "$source_dir" "$asset_dir" "$target"
archive="$asset_dir/ck-occipital-$target.zip"
result=$(xcrun notarytool submit "$archive" \
  --key "$APP_STORE_CONNECT_API_KEY_PATH" \
  --key-id "$APP_STORE_CONNECT_API_KEY_ID" \
  --issuer "$APP_STORE_CONNECT_API_ISSUER_ID" \
  --output-format json --wait)
printf '%s\n' "$result"
if [[ $(printf '%s\n' "$result" | python -c 'import json, sys; print(json.load(sys.stdin)["status"])') != Accepted ]]; then
  echo "notarization was not accepted for $archive" >&2
  exit 1
fi
# A ZIP can't carry a stapled ticket reliably. Once notarization is accepted,
# Gatekeeper can still find the ticket online.
if ! xcrun stapler staple "$archive"; then
  echo "::warning title=staple-pending::$archive is notarized but not stapled"
fi
