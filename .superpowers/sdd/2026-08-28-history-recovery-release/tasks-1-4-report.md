# Tasks 1–4 integrated milestone report

Date: 2026-08-31
Branch: `feature/smartcat-implementation`

## Delivered

- OS credential-backed 32-byte key, AES-256-GCM v1 envelopes, fresh nonces, row/column AAD, zeroized owned key/plaintext buffers, and no plaintext fallback or token copy.
- WAL/foreign-key/secure-delete SQLite bootstrap, encrypted history CRUD/pagination/purge, 30-day default retention, and no plaintext source/result/display-name/path-preview columns.
- Shared persistent secret mode on text, popup, capture, and document surfaces. The backend rejects secret history inserts; secret document checkpoints never become durable.
- Explicit job stages, encrypted seven-day recovery, backend-only document retention handoff, and Continue/Delete/Later recovery UI.
- Canonical encrypted option snapshot/hash covering source/target, resolved profile, model, quality, applicable glossary, and complete format/output options. Resume requires both source fingerprint and option hash to match.
- Canonical-root UUID-child cleanup with symlink rejection, capture-copy cleanup, typed diagnostic allowlist, privacy scanner, and recording policy.

## Commands and results

- `pnpm build`: first run found two `useRef` initialization errors; after correction PASS, 63 modules, Vite 745ms.
- `cargo fmt --manifest-path src-tauri/Cargo.toml`: PASS.
- `cargo check --manifest-path src-tauri/Cargo.toml --lib` using the shared D-drive target: existing cache attempts failed before compilation due incomplete downloaded crate sources; a clean D-drive Cargo home then required the Rust toolchain on build-script PATH. Product compilation found one `MemoryKeyStore` array dereference error. After correction the incremental check PASS in 13.55s.
- `pnpm privacy:check`: PASS, 4 files.
- `pnpm records:test`: PASS, 16 tests, 0 failures.
- `pnpm records:check`: PASS on the final product and required-record changes.
- `git diff --check`: PASS immediately before commit.

## Speed ruling and concerns

No new failing tests, failure injection, full regression, Playwright, installed-app, crash/restart, or long document smoke was added or run. Existing tests were preserved. Follow-up acceptance must cover real Windows Credential Manager/macOS Keychain lock and deletion behavior, SQLite/WAL canary inspection, changed-source/changed-settings restart recovery, and Windows reparse point/macOS symlink cleanup attacks.

## Commit

Milestone commit SHA: pending

## Review fix round 1 — 2026-08-31

- Document stage and completed-batch callbacks now synchronously commit the encrypted recovery row before UI progress is emitted. The persisted payload carries the sequential JobStage state, completed stable units, source fingerprint, canonical option snapshot/hash, and encrypted translated-result references. Secret jobs use the same checkpoint cadence in memory only.
- Document and capture plaintext temporary data now share CleanupService's per-user private root and UUID direct-child layout. Completion/cancellation and the seven-day startup purge remove leftovers; symlinks and Windows reparse points are rejected.
- Successful completion/resume deletes current and consumed recovery rows. History retention runs at startup/save/list, pagination uses `(created_at,id)`, migrations use one transaction plus versioned `user_version`, and actual glossary application shares the canonical snapshot selector.
- OS-key Base64 and decoded buffers are Zeroizing on all exits; keyring failure displays a content/path-free native boot error. Typed allowlist diagnostics are emitted by history/job/cleanup/document/capture paths. The privacy gate fails closed on missing, empty, oversized, or symlink inputs and scans a tracked sanitized captured log. RecoveryPrompt shows checkpoint age.

Verification so far: Rust formatter PASS; `pnpm build` PASS (63 modules, Vite 1.27s). The warm D cache was confirmed, but the offline Rust check was stopped after 60 seconds while dependencies were still compiling, per the speed limit; it had not reached product compilation, so no Rust PASS is claimed. No new failing tests, failure injection, full regression, or long smoke was added or run.

Final short gates: `pnpm privacy:check` PASS (5 files); `pnpm records:test` PASS (16/16); `pnpm records:check` PASS; `git diff --cached --check` PASS. Two preliminary Cargo environment attempts failed before compilation because the first offline cache lacked `aead` and its online repair exposed an incomplete `aes` source. The requested read-only check then confirmed `aead` in the shared warm cache; that correct offline check made dependency compilation progress but was stopped at the 60-second ceiling before the product crate, with no product diagnostic emitted. No credential or Codex token was copied into any command, record, or store.
