#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Darwin ]] || { echo 'macOS acceptance must run on macOS.' >&2; exit 1; }
dmg="${1:?usage: acceptance.sh /path/to/SmartCAT.dmg [--ci-ephemeral]}"; mode="${2:-}"
[[ -f "$dmg" && "$dmg" == *.dmg ]] || { echo 'A DMG artifact is required.' >&2; exit 1; }
if [[ "$mode" == --ci-ephemeral && ( "${CI:-}" != true || "${GITHUB_ACTIONS:-}" != true ) ]]; then echo '--ci-ephemeral requires a GitHub Actions ephemeral runner.' >&2; exit 1; fi
[[ -z "$mode" || "$mode" == --ci-ephemeral ]] || { echo 'unknown acceptance mode' >&2; exit 1; }
root="$(mktemp -d "${TMPDIR:-/tmp}/smartcat-release-acceptance.XXXXXX")"; mount="$root/mount"; app_root="$root/app"; data="$root/test-data"
app_data="$HOME/Library/Application Support/com.smartcat.translate"
mkdir -p "$mount" "$app_root" "$data/home" "$data/tmp"
cleanup() { hdiutil detach "$mount" -quiet 2>/dev/null || true; if [[ "$mode" == --ci-ephemeral ]]; then [[ "$app_data" == "$HOME/Library/Application Support/com.smartcat.translate" ]] && rm -rf -- "$app_data"; case "$root" in "${TMPDIR:-/tmp}"/smartcat-release-acceptance.*) rm -rf -- "$root";; *) echo 'refusing unsafe cleanup' >&2; exit 1;; esac; fi; }
trap cleanup EXIT
hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$mount"
app="$(find "$mount" -maxdepth 1 -name '*.app' -print -quit)"; [[ -n "$app" ]] || { echo 'App bundle missing.' >&2; exit 1; }
ditto "$app" "$app_root/$(basename "$app")"; copied="$app_root/$(basename "$app")"
if [[ "$mode" != --ci-ephemeral ]]; then echo "Dry acceptance passed; app copy retained at $root and was not launched."; exit 0; fi
codesign --verify --deep --strict --verbose=2 "$copied"
spctl --assess --type execute --verbose=4 "$copied"
xcrun stapler validate "$copied"
xcrun stapler validate "$dmg"
executable="$(find "$copied/Contents/MacOS" -maxdepth 1 -type f -perm -111 -print -quit)"; [[ -n "$executable" ]] || { echo 'App executable missing.' >&2; exit 1; }
[[ ! -e "$app_data" ]] || { echo 'GitHub runner app-data path was not clean before acceptance.' >&2; exit 1; }
"$executable" >"$root/app.log" 2>&1 &
pid=$!; deadline=$((SECONDS + 10))
while [[ $SECONDS -lt $deadline ]] && kill -0 "$pid" 2>/dev/null; do sleep 0.25; done
kill -0 "$pid" 2>/dev/null || { echo 'App exited while exercising the real Keychain/default app-data startup path.' >&2; exit 1; }
kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true
[[ "$app_data" == "$HOME/Library/Application Support/com.smartcat.translate" ]] || { echo 'Refusing cleanup outside exact SmartCAT app-data path.' >&2; exit 1; }
echo 'CI ephemeral macOS signature, notarization, staple, real Keychain/default app-data, and stable startup assertions passed.'
