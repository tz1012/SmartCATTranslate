# Workspace Continuity, History, and Notification Fixes Implementation Plan

> **For Codex:** Execute this plan task-by-task with test-driven development and verify each completed behavior before moving on.

**Goal:** Keep text and document work intact across navigation, make notices dismissible, restore reliable history persistence, and collapse the text workspace footer to one compact line.

**Architecture:** Preserve stateful translation workspaces by keeping their React trees mounted and hiding inactive panels. Treat notification acknowledgement as UI session state keyed by notification ID. Make text-history persistence an explicit retryable operation whose success, rather than its attempt, marks a job saved. Keep the footer actions and status in one flex row.

**Tech Stack:** React 19, TypeScript, Vitest, Testing Library, Tauri 2, Rust/rusqlite, CSS.

---

### Task 1: Lock navigation continuity with regression tests

**Files:**
- Modify: `src/app/App.test.tsx`
- Modify: `src/app/App.tsx`

- [x] Add a test that types source text, visits History, returns to Text, and sees the same source.
- [x] Add a test that selects a document, visits Text, returns to Documents, and sees the same selected file.
- [x] Run the focused App tests and confirm both fail because inactive panels are unmounted.
- [x] Render stateful workspaces once and toggle their `hidden` state; keep settings/history mounted only when active.
- [x] Re-run the focused tests and confirm they pass.

### Task 2: Add per-notification acknowledgement

**Files:**
- Modify: `src/app/AppNotificationPopover.tsx`
- Modify: `src/app/App.tsx`
- Modify: `src/app/App.test.tsx`
- Modify: `src/styles.css`

- [x] Add a test that opens a notice, clicks `확인`, and verifies the notice and badge count disappear.
- [x] Confirm the focused test fails because notifications have no acknowledgement control.
- [x] Add localized dismiss labels and an `onDismiss` callback per notification.
- [x] Track dismissed notification IDs for the running app session and close the empty popover.
- [x] Re-run the focused tests and confirm they pass.

### Task 3: Make text-history saves observable and retryable

**Files:**
- Modify: `src/features/translation/TextWorkspace.test.tsx`
- Modify: `src/features/translation/TextWorkspace.tsx`
- Modify: `src/styles.css`

- [x] Add a test proving a completed normal translation invokes `save_history_record` once with the translated payload.
- [x] Add a test that forces `save_history_record` to reject, shows a safe error, retries, and succeeds.
- [x] Confirm the retry test fails with the current swallowed error and pre-emptive saved marker.
- [x] Mark a job saved only after persistence resolves; retain every pending payload by job ID for an explicit retry button.
- [x] Re-run focused translation tests and confirm success without duplicate saves.

### Task 4: Collapse the footer into one compact line

**Files:**
- Modify: `src/features/translation/TextWorkspace.test.tsx`
- Modify: `src/features/translation/TextWorkspace.tsx`
- Modify: `src/styles.css`

- [x] Add a structural test that the status text is inside the footer action row.
- [x] Confirm it fails with the existing separate grid row.
- [x] Move the status beside the actions and style the row as a compact wrapping flex line with no reserved status height.
- [x] Re-run focused tests and perform a production build.

### Task 5: Version, release integration, and local installation

**Files:**
- Modify: `package.json`
- Modify: `pnpm-lock.yaml`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `CHANGELOG.md`

- [x] Bump the application version from `0.1.5` to `0.1.6` and record the four fixes.
- [x] Run focused tests, the full frontend suite, TypeScript/Vite build, and Rust tests/checks.
- [x] Request code review, address findings, and repeat verification.
- [ ] Commit only intended tracked changes, push a branch, open and merge a pull request after checks pass.
- [ ] Build the Windows installer, install it silently over the local app, restart it, and verify executable/registry version `0.1.6`.
