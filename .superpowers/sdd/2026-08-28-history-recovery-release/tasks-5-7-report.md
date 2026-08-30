# Tasks 5–7 release implementation report

Date: 2026-08-31
Branch: `feature/smartcat-implementation`

## Delivered

- Manual-only updater UI and Rust commands: version/release notes/date/size display, Later without download, exact-version 15-minute one-use check/install tokens, explicit download, Tauri whole-payload signature verification with progress, distinct network/signature/install errors, and a separate restart confirmation before install/restart.
- No background check/download/install/restart and no silent rollback. Private pending state retains from/target versions and both GitHub Release installer references; `last-known-good.json` is written only when the updated version's hydrated frontend reports main-window readiness and current equals target.
- Local/dev updater is committed disabled and commands fail with `updater_not_configured`. `scripts/configure-updater.mjs` runs only in a protected release checkout, requires a valid `GITHUB_REPOSITORY` and real `TAURI_UPDATER_PUBLIC_KEY`, and materializes only `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`.
- No fake endpoint/public key, updater private key, PFX/PKCS#12, certificate password, Apple credential, or signing certificate is committed or generated.
- CI/release policy for Windows x64 MSI+NSIS, macOS 15 Intel app+DMG, and macOS 14 Apple Silicon app+DMG with pinned pnpm 10.17.1 and Rust 1.95.0. Frontend/Rust/records/privacy/runtime-asset gates precede Tauri packaging. PR artifacts are explicitly `UNSIGNED`.
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
- Actual installed-app smoke on macOS 15 Intel and macOS 14 Apple Silicon remains CI-required. Windows local unsigned smoke and packaging scripts were implemented but not run.
- Public release is blocked until the incomplete Rust compile check finishes in CI and every unchecked release checklist item is evidenced.

## Commit

Initial Tasks 5–7 milestone: `113aad9 feat: add consent-based signed release pipeline`.

## Publication fix round 1

- Replaced the retired Intel runner with `macos-15-intel` in CI, release workflow, plan, checklist and records. The positive workflow checker rejects the retired label.
- Added an exact existing `refs/tags/app-vSEMVER` resolver that rejects branch/SHA dispatch inputs and proves tag, `package.json`, Cargo, and Tauri versions match before any protected Environment secret is accessible. Every package/publish checkout uses the verified commit SHA.
- Removed setup-time LKG marking. Install now writes versioned pending state; a one-shot frontend hydration/main-window-ready handshake marks LKG only for the target version and clears pending/rollback. Recovery is offered only when the previous version remains active for a newer valid pending target. Restart authorization is backend-issued, bound to version/install token, short-lived, and consumed once by install.
- Protected secret checks and use are platform-scoped. Each package leg signs a disposable canary with the updater private key and verifies it against the configured public key through maintained `minisign-verify`; temporary canary material is deleted and never uploaded.
- Signed release legs now execute platform acceptance before artifact upload/publish: Windows requires Authenticode `Valid`, silent MSI install, real Credential Manager/default app data, stable main-window startup, uninstall, exact app-data cleanup, and an unchanged Documents snapshot; macOS requires preserved app copy, strict codesign, Gatekeeper, app/DMG staple validation, real Keychain/default app data, and stable startup. Actual mode requires both GitHub Actions markers; local default scripts do not install or launch.
- macOS app directories are preserved with `ditto` and updater tar/signature plus DMG are uploaded. Production npm/Cargo SBOM components include license expressions; actual discovered license/notice texts and an inventory are bundled, and unreviewed missing metadata/text fails release.
- The user-directed speed ruling overrides the plan's request for new negative/E2E tests. None were added or run; that reduced proof depth is explicitly retained as a verification gap.

### Fix-round verification evidence

- Rust formatting: PASS.
- `pnpm build`: PASS, 64 modules, 1.19 seconds.
- `cargo check --locked --manifest-path src-tauri/Cargo.toml --lib` using shared D caches: first bounded run reached the product crate at about 129 seconds and found one pre-existing `Zeroizing<[u8; 32]>` coercion error in `OsKeyStore`; the direct `&mut *key` correction was formatted and the one permitted incremental recheck PASSed in 35.94 seconds. Rust library PASS is claimed for this checkout.
- `pnpm release:workflow:check`: PASS after both workflow YAML files parsed and semantic checks positively confirmed the supported matrix, verified-SHA gate ordering, protected environment dependency, platform-scoped secrets, updater keypair preflight, platform trust/acceptance commands, preserved macOS app archive, and all-leg draft dependency.
- Windows production dependency license bundle: PASS, 435 components. macOS Intel and ARM: PASS, 434 components each. npm/Cargo SBOM generation with license expressions: PASS. Actual discovered license/notice files are copied; 26 version-specific published-crate omissions have explicit reviewed expression/reason entries and no implicit exception.
- Runtime/Codex/font checksum and PDFium non-bundling: PASS.
- PowerShell acceptance parser, Bash syntax, and release Node script syntax: PASS.
- Records: PASS, 16/16; privacy scan: PASS, 5 files; records check: PASS; `git diff --check`: PASS.

### Still unverified and publication-blocking

- This checkout still has no Git remote or existing `app-vSEMVER` tag, so the exact-tag positive gate could not be exercised locally; its workflow and script were statically validated. It will fail closed before protected secrets if repository/tag/version state is wrong.
- Protected updater private/public keys, Windows certificate, Apple signing/notarization credentials are unavailable locally. The canary pairing, signed MSI/NSIS, notarized/stapled macOS app/DMG, updater artifacts/signatures, CI acceptance, provenance attestation, and draft release are not claimed.
- Actual macOS 15 Intel and macOS 14 Apple Silicon installed-app acceptance remains required in CI. Windows CI silent install/launch/uninstall was implemented but not run on the local unsigned checkout.
- Per the latest user ruling, no new negative updater/recovery tests, new E2E tests, failure injection, Playwright, or full regression suite was added or run. Existing tests remain intact; missing new plan tests remain a ledger verification gap.

Fix-round commit: the commit containing this report (`fix: harden release publication gates`).

## Publication fix round 2

- Windows update metadata now selects only Tauri v2 signed updater archives with adjacent signatures: `.nsis.zip` is preferred and `.msi.zip` is the documented fallback. Raw `.exe`/`.msi` feed entries are forbidden. This follows the [official Tauri v2 updater artifact documentation](https://v2.tauri.app/plugin/updater/).
- Removed every production acceptance switch, fixed encryption key, app-data override, and ready-marker path. The signed binary always opens the operating system credential store and default per-user app-data directory. Actual acceptance is allowed only when both `CI=true` and `GITHUB_ACTIONS=true`; local mode only extracts/copies. The disposable GitHub runner exercises real Credential Manager/Keychain failure-closed startup, then removes only the exact SmartCAT app-data directory.
- Replaced the single animation-frame LKG signal with successful lifecycle, account, settings, privacy, history and recovery loads followed by a 1.5-second stability interval. Transient frontend/backend failures retry and leave pending state untouched. Backend `healthy_marked` is set only on successful command paths after LKG write and pending/rollback clear succeed.
- Normalized the five legacy slash expressions to `MIT OR Apache-2.0`. Every dependency/asset expression is parsed by maintained `spdx-expression-parse`; generated npm/Cargo BOM JSON is validated against CycloneDX 1.6 through the official OWASP CycloneDX JavaScript library before it is written.
- `latest.json` is created inside `release-assets` before the SHA-256 inventory. The inventory includes the manifest and intentionally excludes only itself, feeds build provenance attestation, and the same files are uploaded. Workflow policy parses both YAML files and directly validates these script semantics/order while rejecting a raw Windows executable feed or production acceptance backdoor.

### Round-2 verification evidence

- Rust formatting: PASS. Warm shared-cache `cargo check --locked --manifest-path src-tauri/Cargo.toml --lib`: PASS in 30.56 seconds, within the 120-second ceiling, with zero diagnostics.
- `pnpm build`: PASS, 64 modules, 1.19 seconds.
- `pnpm release:workflow:check`: PASS. Both workflow YAML documents parsed; direct semantic checks confirmed signed updater ZIP patterns/adjacent signatures and NSIS preference, rejected raw Windows feed patterns, enforced manifest/checksum/attestation/upload order and non-self checksum, required GitHub Actions acceptance guards, rejected production acceptance bypasses, and required the six LKG probes/stability plus SPDX/CycloneDX validators.
- SPDX/license evidence: PASS — Windows 435 components, macOS Intel 434, macOS ARM 434. Legacy slash-list metadata is normalized to `OR` only when every token has SPDX-ID syntax, then every result is parsed; other invalid expressions fail. npm and all three target Cargo SBOMs passed official CycloneDX 1.6 structural validation.
- Runtime/Codex/font checksum and PDFium non-bundling: PASS. PowerShell acceptance parse, Bash syntax, and release Node script syntax: PASS.
- Records: PASS, 16/16; privacy: PASS, 5 files; records check: PASS; `git diff --check`: PASS. Static search found no production/test acceptance environment switch, fixed key, or ready-marker path.

### Round-2 external blockers

- No remote, release tag, protected updater/Windows/Apple secrets, or signed Tauri v2 updater archives exist locally. Actual updater ZIP selection/signature, real Credential Manager/Keychain acceptance, Windows install/uninstall, macOS notarization/stapling, manifest attestation, draft release, and publication remain unverified and fail-closed CI requirements.
- No new failure, negative updater/LKG, E2E, Playwright, or full regression tests were added or run per the latest user instruction. Existing tests remain unchanged; the missing new tests remain a ledger verification gap.

Round-2 commit: the commit containing this report (`fix: close updater and acceptance publication gaps`).
