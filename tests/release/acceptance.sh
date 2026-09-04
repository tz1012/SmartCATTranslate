#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Darwin ]] || { echo 'macOS acceptance must run on macOS.' >&2; exit 1; }
dmg="${1:?usage: acceptance.sh /path/to/SmartCAT.dmg [--ci-ephemeral]}"; mode="${2:-}"
[[ -f "$dmg" && "$dmg" == *.dmg ]] || { echo 'A DMG artifact is required.' >&2; exit 1; }
if [[ "$mode" == --ci-ephemeral && ( "${CI:-}" != true || "${GITHUB_ACTIONS:-}" != true ) ]]; then echo '--ci-ephemeral requires a GitHub Actions ephemeral runner.' >&2; exit 1; fi
[[ -z "$mode" || "$mode" == --ci-ephemeral ]] || { echo 'unknown acceptance mode' >&2; exit 1; }
app_data_owned=false; copied_by_this_run=false; root_owned=false; copied=''
runner_home="$(dscl . -read "/Users/$(id -un)" NFSHomeDirectory | sed 's/^NFSHomeDirectory: //')"
runner_app_support="$(cd "$runner_home/Library/Application Support" && pwd -P)"
expected_app_data="$runner_app_support/com.smartcat.translate"
root="$(mktemp -d "${TMPDIR:-/tmp}/smartcat-release-acceptance.XXXXXX")"; mount="$root/mount"; app_root="$root/app"; data="$root/test-data"
root_owned=true
mkdir -p "$mount" "$app_root" "$data/home" "$data/tmp"
cleanup() {
  hdiutil detach "$mount" -quiet 2>/dev/null || true
  if [[ "$app_data_owned" == true && -e "$expected_app_data" ]]; then
    [[ ! -L "$expected_app_data" ]] || { echo 'refusing app-data symlink cleanup' >&2; return 1; }
    [[ "$(dirname "$expected_app_data")" == "$runner_app_support" && "$(basename "$expected_app_data")" == com.smartcat.translate ]] || { echo 'refusing cleanup outside exact runner SmartCAT app-data path' >&2; return 1; }
    rm -rf -- "$expected_app_data"
  fi
  if [[ "$mode" == --ci-ephemeral && "$copied_by_this_run" == true && -n "$copied" && -e "$copied" ]]; then
    [[ "$(dirname "$copied")" == "$app_root" ]] || { echo 'refusing cleanup outside owned app-copy root' >&2; return 1; }
    rm -rf -- "$copied"
  fi
  if [[ "$root_owned" == true && "$mode" == --ci-ephemeral ]]; then case "$root" in "${TMPDIR:-/tmp}"/smartcat-release-acceptance.*) rm -rf -- "$root";; *) echo 'refusing unsafe cleanup' >&2; return 1;; esac; fi
}
trap cleanup EXIT
hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$mount"
app="$(find "$mount" -maxdepth 1 -name '*.app' -print -quit)"; [[ -n "$app" ]] || { echo 'App bundle missing.' >&2; exit 1; }
copied="$app_root/$(basename "$app")"
[[ ! -e "$copied" ]] || { echo 'Owned app-copy destination already exists.' >&2; exit 1; }
if [[ "$mode" == --ci-ephemeral ]]; then
  [[ ! -e "$expected_app_data" ]] || { echo 'GitHub runner app-data path was not clean before acceptance.' >&2; exit 1; }
  set +e; security find-generic-password -s com.smartcat.translate -a local-data-key >/dev/null 2>&1; keychain_status=$?; set -e
  [[ $keychain_status -eq 44 ]] || { [[ $keychain_status -eq 0 ]] && echo 'GitHub runner Keychain already contains the SmartCAT key.' >&2 || echo 'Unable to prove the SmartCAT Keychain item is absent.' >&2; exit 1; }
  app_name="$(basename "$app")"
  [[ ! -e "/Applications/$app_name" && ! -e "$runner_home/Applications/$app_name" ]] || { echo 'GitHub runner already has BYOK Translator installed.' >&2; exit 1; }
fi
ditto "$app" "$copied"; copied_by_this_run=true
if [[ "$mode" != --ci-ephemeral ]]; then echo "Dry acceptance passed; app copy retained at $root and was not launched."; exit 0; fi
codesign --verify --deep --strict --verbose=2 "$copied"
spctl --assess --type execute --verbose=4 "$copied"
xcrun stapler validate "$copied"
xcrun stapler validate "$dmg"
executable="$(find "$copied/Contents/MacOS" -maxdepth 1 -type f -perm -111 -print -quit)"; [[ -n "$executable" ]] || { echo 'App executable missing.' >&2; exit 1; }
"$executable" >"$root/app.log" 2>&1 &
pid=$!; deadline=$((SECONDS + 10))
while [[ $SECONDS -lt $deadline ]] && kill -0 "$pid" 2>/dev/null; do [[ "$app_data_owned" == false && -e "$expected_app_data" ]] && app_data_owned=true; sleep 0.25; done
[[ "$app_data_owned" == true || ! -e "$expected_app_data" ]] || app_data_owned=true
kill -0 "$pid" 2>/dev/null || { echo 'App exited while exercising the real Keychain/default app-data startup path.' >&2; exit 1; }
kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true
echo 'CI ephemeral macOS signature, notarization, staple, real Keychain/default app-data, and stable startup assertions passed.'
