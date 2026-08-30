# Tasks 5–7 release implementation report

Date: 2026-08-31
Branch: `feature/smartcat-implementation`

## Delivered

- Manual-only updater UI and Rust commands: version/release notes/date/size display, Later without download, exact-version 15-minute one-use check/install tokens, explicit download, Tauri whole-payload signature verification with progress, distinct network/signature/install errors, and a separate restart confirmation before install/restart.
- No background check/download/install/restart and no silent rollback. Private update state retains previous GitHub Release installer instructions and writes `last-known-good.json` only after the main window setup path is reached.
- Local/dev updater is committed disabled and commands fail with `updater_not_configured`. `scripts/configure-updater.mjs` runs only in a protected release checkout, requires a valid `GITHUB_REPOSITORY` and real `TAURI_UPDATER_PUBLIC_KEY`, and materializes only `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`.
- No fake endpoint/public key, updater private key, PFX/PKCS#12, certificate password, Apple credential, or signing certificate is committed or generated.
- CI/release policy for Windows x64 MSI+NSIS, macOS 13 Intel app+DMG, and macOS 14 Apple Silicon app+DMG with pinned pnpm 10.17.1 and Rust 1.95.0. Frontend/Rust/records/privacy/runtime-asset gates precede Tauri packaging. PR artifacts are explicitly `UNSIGNED`.
- Protected `release-signing` environment preflight, Windows certificate import/thumbprint signing, macOS temporary keychain signing plus notarization credentials and cleanup, updater artifact signing, all-leg-dependent draft release, checksums, npm/Cargo CycloneDX SBOMs, bundled licenses, run/commit provenance and GitHub attestation.
- Literal Codex runtime archive pins and Noto font/license checksums are verified. Built sidecar checksum is required in release jobs. PDFium is rejected from bundled resources.
- Non-destructive local unsigned packaging and installed-app acceptance scripts use disposable app/test-data roots. Windows uses an MSI administrative image and verifies user Documents hashes are unchanged; macOS mounts a DMG read-only and copies the app into a disposable root. The short manual checklist covers login, text, hotkey, capture, document, history, recovery, and updater consent.

## Verification evidence

- `pnpm build`: PASS, TypeScript and Vite, 64 modules, 781 ms.
- Rust formatting: PASS after using the installed 1.95.0 toolchain directly.
- One shared-D `cargo check --manifest-path src-tauri/Cargo.toml --lib`: stopped at the required 180-second ceiling while compiling dependencies including Tauri/Wry/rustls. It had not reached the `smartcat-translate` product crate and emitted zero product code diagnostics. Rust PASS is **not** claimed. The run updated `src-tauri/Cargo.lock` with `tauri-plugin-updater` 2.10.1 and its locked dependencies.
- `pnpm release:workflow:check`: PASS. Both GitHub workflow YAML files parsed with `yaml` 2.8.1 and direct positive policy checks confirmed jobs, matrices, gate ordering, protected signing, draft dependency, evidence, release-only updater configuration, and absence of fake/private material.
- `node scripts/verify-runtime-assets.mjs`: PASS; Codex targets/pins and Noto hashes verified, PDFium not bundled.
- `pnpm records:test`: PASS, 16/16.
- `pnpm privacy:check`: PASS, 5 files.
- `pnpm records:check`: PASS.
- `git diff --check`: PASS.

Per the latest speed instruction, no new failure reproduction, failure injection, full regression, Playwright, cargo test/clippy, long document test, package build, or installed-app smoke was run. Existing tests remain present.

## Required external verification before publication

- This checkout has no Git remote. The user must select/push the real GitHub repository; the endpoint is intentionally not guessed.
- The protected GitHub `release-signing` Environment and all updater/Windows/Apple secrets listed in `docs/release-checklist.md` are absent/unverified locally. No signing key or certificate was generated because the user did not authorize key creation.
- Windows signed MSI/NSIS, macOS signed/notarized app/DMG, updater signatures/latest manifest, GitHub provenance attestation and draft release remain CI outputs.
- Actual installed-app smoke on macOS 13 Intel and macOS 14 Apple Silicon remains CI-required. Windows local unsigned smoke and packaging scripts were implemented but not run.
- Public release is blocked until the incomplete Rust compile check finishes in CI and every unchecked release checklist item is evidenced.

## Commit

Milestone commit: pending
