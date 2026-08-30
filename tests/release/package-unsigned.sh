#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Darwin ]] || { echo 'macOS packaging must run on macOS.' >&2; exit 1; }
repo="$(cd "$(dirname "$0")/../.." && pwd -P)"
target="${SMARTCAT_TARGET:-$(uname -m | sed 's/x86_64/x86_64-apple-darwin/;s/arm64/aarch64-apple-darwin/')}"
output="${1:-$(mktemp -d "${TMPDIR:-/tmp}/smartcat-unsigned-package.XXXXXX")}"
mkdir -p "$output/test-data"
export HOME="$output/test-data/home" TMPDIR="$output/test-data/tmp" APPLE_SIGNING_IDENTITY=-
mkdir -p "$HOME" "$TMPDIR"
cd "$repo"
pnpm release:assets:verify
pnpm runtime:build -- --target "$target"
pnpm tauri build --config src-tauri/tauri.runtime.conf.json --target "$target" --bundles app,dmg
mkdir -p "$output/UNSIGNED-bundles"
cp -R "src-tauri/target/$target/release/bundle/." "$output/UNSIGNED-bundles/"
find "$output/UNSIGNED-bundles" -type f -exec shasum -a 256 {} \; > "$output/SHA256SUMS"
printf 'Unsigned macOS prerelease staged at %s. No user document directory was modified.\n' "$output"
