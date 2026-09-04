# BYOK Translator Rebrand Implementation Plan

> **For Codex:** Execute this plan task-by-task with test-driven development and verification before completion.

**Goal:** Rename the installed and user-visible product to BYOK Translator, replace the application icon, preserve existing user data and update compatibility, and deliver the change locally and through GitHub.

**Architecture:** Keep the stable bundle identifier (`com.smartcat.translate`), internal executable/package names, and GitHub updater repository unchanged so existing settings, history, shortcuts, and update discovery continue to work. Change only user-facing product metadata, UI strings, installer display names, release metadata, and generated icon assets. Ship the rebrand as version 0.2.0.

**Tech Stack:** React, TypeScript, Vitest, Tauri 2, Rust, SVG/PNG/ICO/ICNS assets, GitHub Actions.

---

### Task 1: Lock user-visible branding with tests

**Files:**
- Modify: `src/app/AppMenuOverlay.test.tsx`
- Modify: `src/app/App.test.tsx`

1. Add assertions for the BYOK Translator app information and absence of the old visible product name.
2. Run the focused tests and confirm they fail against the current branding.

### Task 2: Rename product metadata and visible strings

**Files:**
- Modify: `index.html`
- Modify: `src/app/AppMenuOverlay.tsx`
- Modify: `src/features/hotkeys/HotkeySettings.tsx`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Info.plist`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/lifecycle.rs`
- Modify: `.github/workflows/release.yml`
- Modify: `scripts/generate-sbom.mjs`
- Modify: `scripts/release-evidence.mjs`

1. Replace user-visible SmartCAT Translate naming with BYOK Translator.
2. Preserve internal executable names, bundle identifier, storage/keychain identifiers, and GitHub repository URLs.
3. Run focused tests until green.

### Task 3: Replace and regenerate application icons

**Files:**
- Modify: `src-tauri/icons/icon.svg`
- Regenerate: `src-tauri/icons/*` platform icon assets

1. Replace the existing letterform with a high-contrast key-and-translation vector mark suitable at 16–512 px.
2. Run `pnpm icons:generate` and inspect the generated source and small Windows icon.
3. Run runtime asset verification.

### Task 4: Version, records, and regression verification

**Files:**
- Modify: `package.json`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `CHANGELOG.md`
- Modify: `PROJECT_LOG.txt`

1. Synchronize version 0.2.0 across manifests and lockfile.
2. Record the compatibility boundary and rebrand details.
3. Run frontend tests, Rust tests, production build, E2E tests, record/privacy/runtime/workflow gates, and diff checks.

### Task 5: Deliver locally and through GitHub

1. Build a Windows NSIS installer from the verified commit while keeping the updater configuration compatible.
2. Hash the installer, preserve the existing settings/history data files, install into the existing local installation, relaunch, and verify version/process/product metadata.
3. Commit only product changes, push a feature branch, open a pull request, verify CI, and merge.
4. Publish the public 0.2.0 release with the verified Windows installer and release notes; verify tag target, anonymous download, file size, and SHA-256.
