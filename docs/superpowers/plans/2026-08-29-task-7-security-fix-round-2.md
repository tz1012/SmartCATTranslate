# Task 7 Security Fix Round 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the remaining Task 7 Critical/Important findings without weakening the stable Codex 0.144.4 protocol boundary.

**Architecture:** SmartCAT constructs one version-pinned, schema-valid isolated Codex home; launches the official app server only through an inherited OS sandbox; and treats every unexpected request or failed cancellation as a fatal runtime condition. Tauri window ownership is synchronously tombstoned before asynchronous cleanup, so destroyed windows cannot reserve work.

**Tech Stack:** Rust, Tokio, Tauri 2, serde/toml, pinned Codex 0.144.4 JSON schema and binary, Windows Codex sandbox, macOS Seatbelt.

---

### Task 1: Pin and validate the isolated Codex configuration

**Files:** `src-tauri/src/codex/process.rs`, `src-tauri/tests/codex_runtime.rs`, `src-tauri/resources/codex-0.144.4-config.schema.json`, `src-tauri/Cargo.toml`

- [x] Add failing tests for 0.144.4 schema validation, hostile inherited configuration/environment, strict ChatGPT credential shape, and symlink/reparse/oversize rejection.
- [x] Replace guessed keys with the smallest valid 0.144.4 allowlisted configuration and canonical credential import.
- [x] Add an opt-in real pinned-binary integration test and run it against the verified release binary.

### Task 2: Enforce process-wide OS filesystem confinement

**Files:** `src-tauri/src/codex/process.rs`, `src-tauri/tests/codex_runtime.rs`, `src-tauri/tests/translation_coordinator.rs`

- [x] Add failing policy-construction tests for Windows and the supported turn policy without `readableRoots`; preserve fail-closed cfg branches for macOS and unsupported targets.
- [x] Launch app-server only through the platform sandbox, with explicit app-owned paths and cleared inherited environment; no unsandboxed fallback exists.
- [ ] Run the combined Windows acceptance. The outside-secret denial and allowed-root write pass, but pinned 0.144.4 App Server initialization is blocked by its `CODEX_HOME` canonicalization under AppContainer; see D-022 and the Task 7 report. macOS cannot be compile-run on this Windows host.

### Task 3: Make cancellation and server requests fatal when safety is uncertain

**Files:** `src-tauri/src/codex/transport.rs`, `src-tauri/src/codex/translation.rs`, `src-tauri/tests/codex_transport.rs`, `src-tauri/tests/translation_coordinator.rs`

- [x] Add failing tests for every server request ID shape with no subscribers and for interrupt timeout/error/malformed response.
- [x] Terminate transport on every inbound server request and taint/terminate the backend whenever interrupt acknowledgement is uncertain.
- [x] Prove no new job can start after taint.

### Task 4: Close the Tauri destroy scheduling race

**Files:** `src-tauri/src/commands/translation.rs`, `src-tauri/src/app_state.rs`, `src-tauri/src/lib.rs`, `src-tauri/tests/translation_coordinator.rs`

- [x] Add a failing exact scheduling-race test using a synchronous owner tombstone.
- [x] Install the tombstone in the real Tauri destroy callback before spawning cancellation cleanup and make command start atomically observe it.

### Task 5: Verify, record, and commit

**Files:** `PROJECT_LOG.txt`, `DECISIONS.txt`, `CHANGELOG.md`, `task-7-report.md`

- [x] Run focused RED/GREEN evidence, full Rust/frontend/build/format/check/pinning/records/range gates, and privacy scans.
- [x] Record addressed/open findings, exact test counts, platform sandbox boundary, and self-review.
- [x] Commit the round-two security fix as one scoped commit.
