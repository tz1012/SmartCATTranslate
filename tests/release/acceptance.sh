#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Darwin ]] || { echo 'macOS acceptance must run on macOS.' >&2; exit 1; }
dmg="${1:?usage: acceptance.sh /path/to/SmartCAT.dmg [--run-short-smoke]}"
[[ -f "$dmg" && "$dmg" == *.dmg ]] || { echo 'A DMG artifact is required.' >&2; exit 1; }
root="$(mktemp -d "${TMPDIR:-/tmp}/smartcat-release-acceptance.XXXXXX")"
mount="$root/mount"; app_root="$root/app"; data="$root/test-data"; mkdir -p "$mount" "$app_root" "$data/home" "$data/tmp"
cleanup() { hdiutil detach "$mount" -quiet 2>/dev/null || true; case "$root" in "${TMPDIR:-/tmp}"/smartcat-release-acceptance.*) rm -rf -- "$root";; *) echo 'refusing unsafe cleanup' >&2;; esac; }
trap cleanup EXIT
hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$mount"
app="$(find "$mount" -maxdepth 1 -name '*.app' -print -quit)"; [[ -n "$app" ]] || { echo 'App bundle missing.' >&2; exit 1; }
cp -R "$app" "$app_root/"
copied="$app_root/$(basename "$app")"
codesign --verify --deep --strict --verbose=2 "$copied"
codesign -dv --verbose=4 "$copied" 2> "$root/codesign.txt"
spctl --assess --type execute --verbose=4 "$copied" > "$root/gatekeeper.txt" 2>&1 || true
if [[ "${2:-}" == --run-short-smoke ]]; then
  HOME="$data/home" TMPDIR="$data/tmp" open -W -n "$copied" --args --smartcat-acceptance-root "$data" &
  echo 'Perform docs/release-smoke-checklist.md, then close the app.'
  wait
fi
echo 'macOS acceptance used only a disposable app/test-data root; actual Intel and Apple Silicon runs remain required in CI.'
