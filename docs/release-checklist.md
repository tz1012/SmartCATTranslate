# Release checklist

## One-time GitHub setup

- [ ] Create or select the GitHub repository and push the complete history; this checkout currently has no remote, so no repository name was invented.
- [ ] Create a protected GitHub Environment named `release-signing`, restrict it to release tags/approved reviewers, and add the secrets below.
- [ ] Add `TAURI_UPDATER_PUBLIC_KEY` (the real Minisign public key) and `TAURI_SIGNING_PRIVATE_KEY` plus `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (the corresponding private updater signing key/password).
- [ ] Add `WINDOWS_CERTIFICATE` (base64 PFX), `WINDOWS_CERTIFICATE_PASSWORD`, and `WINDOWS_CERTIFICATE_THUMBPRINT` for Windows code signing.
- [ ] Add `APPLE_CERTIFICATE` (base64 Developer ID Application PKCS#12), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific password), and `APPLE_TEAM_ID` for macOS signing/notarization.
- [ ] Never commit or paste a private updater key, PFX/PKCS#12, password, Apple credential, or notarization key into source, logs, artifacts, or workflow text. This repository does not generate any signing identity.

`scripts/configure-updater.mjs` runs only in the protected release job. It requires GitHub's `GITHUB_REPOSITORY` and the real public key, writes exactly `https://github.com/<owner>/<repository>/releases/latest/download/latest.json` into that ephemeral checkout, and fails when either value is absent or looks invalid. The committed local/development configuration intentionally has no updater endpoint, public key, or update artifacts and the Rust commands return `updater_not_configured`.

## Before tagging

- [ ] Versions match in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`; tag is `app-v<semver>`.
- [ ] The dispatch input names an exact existing `refs/tags/app-vSEMVER`; `scripts/verify-release-ref.mjs` rejects branch/SHA inputs and requires tag/package/Cargo/Tauri versions to match before the protected environment is entered.
- [ ] `CHANGELOG.md`, `PROJECT_LOG.txt`, `DECISIONS.txt`, the implementation report, and this checklist describe the exact release.
- [ ] Frontend build, Rust format/check, records, privacy, runtime/font checksum, workflow policy, and diff gates pass.
- [ ] Codex runtime archives remain pinned to HTTPS GitHub release URLs and literal SHA-256 values; the built sidecar checksum matches its manifest.
- [ ] Noto Sans and its license checksum match. PDFium is not present in resources or bundle configuration.
- [ ] No fake endpoint/key, private secret, auth token, translation content, full user path, or unsigned artifact is labeled as a public release.

## CI and artifact review

- [ ] PR package artifacts are named `UNSIGNED-*`; expected platform trust warnings are recorded and they are never promoted.
- [ ] Release matrix succeeds on Windows x64 (`msi,nsis`), macOS 15 Intel (`app,dmg`), and macOS 14 Apple Silicon (`app,dmg`).
- [ ] The protected job signs a disposable canary and the maintained `minisign-verify` verifier proves the public/private updater key pairing before packaging; canary and signature are deleted and never uploaded.
- [ ] Windows Authenticode status is exactly `Valid` for MSI and installed executable. macOS `codesign --verify --deep --strict`, `spctl --assess`, and `xcrun stapler validate` for both app and DMG all succeed before upload.
- [ ] Tauri updater archives and `.sig` files exist for every platform; `latest.json` contains the exact tag URLs, version, signature, date, notes and byte size.
- [ ] Per the [official Tauri v2 updater artifact contract](https://v2.tauri.app/plugin/updater/), Windows `latest.json` points to a signed `.nsis.zip` (preferred) or adjacent-signature `.msi.zip`, never a raw installer; macOS points to the signed `.app.tar.gz`.
- [ ] `release-assets/latest.json` is generated before `release-assets/SHA256SUMS`; the checksum includes `latest.json` and excludes only the checksum file itself, and that exact checksum inventory feeds provenance attestation before the same files are uploaded.
- [ ] Every npm/Cargo/asset license is accepted by the maintained SPDX parser and both npm/Cargo SBOMs pass the official CycloneDX 1.6 JSON validator before packaging.
- [ ] SHA-256 inventories, npm/Cargo CycloneDX SBOMs with license expressions, actual dependency license/notice texts and reviewed exceptions, commit/run/target provenance records, records/privacy summaries, and GitHub artifact attestation are attached.
- [ ] The draft GitHub Release appears only after all matrix legs succeed. Review it before publishing; never publish directly from an individual matrix leg.

## Short installed-app acceptance

- [ ] Release CI runs `tests/release/acceptance.ps1 -CiEphemeral` only when both `CI=true` and `GITHUB_ACTIONS=true`. It uses the runner's real Credential Manager and default per-user app data, requires a stable main window, then uninstalls and removes only the exact `com.smartcat.translate` app-data directory; Documents hashes remain unchanged. Local default is administrative extraction only.
- [ ] Release CI runs `tests/release/acceptance.sh --ci-ephemeral` under the same two GitHub-runner guards on macOS 15 Intel and macOS 14 Apple Silicon. It exercises the real Keychain/default app data and stable process startup; local default only copies the app and never launches.
- [ ] Confirm the production binary contains no acceptance environment switch, app-data override, fixed storage key, or readiness-marker bypass. Secure-store unavailability must still fail closed.
- [ ] Complete every item in `docs/release-smoke-checklist.md`: login, text, hotkey, capture, document, history, recovery, and updater consent/restart behavior.
- [ ] Verify previous-installer and last-known-good instructions. A rollback is always a user action; the app never silently downloads, installs, restarts, or rolls back.
- [ ] Record artifact hashes, OS/architecture, expected unsigned warnings (if any), tester, time, result, and remaining signing/notarization requirements without user content or secrets.

## Promotion

- [ ] Resolve every failed or unverified item. Missing secrets, signatures, notarization, macOS smoke, or updater metadata block publication.
- [ ] Publish the reviewed draft release, then manually check the installed stable build's update metadata without consenting to another download.
