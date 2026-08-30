# Encrypted History, Job Recovery, and Cross-Platform Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 로컬 암호화 기록과 시크릿 모드, 중단 작업 복구, 개인정보 검사, 사용자 승인 업데이트와 Windows/macOS 서명 배포를 완성한다.

**Architecture:** SQLite에는 검색 가능한 비민감 메타데이터와 AES-256-GCM 암호문만 저장하고 데이터 키는 운영체제 자격 증명 저장소가 보관한다. 작업 상태기계는 단계별 암호화 체크포인트를 남기며 릴리스 파이프라인은 기록, 테스트, 라이선스, 서명과 업데이트 매니페스트를 하나의 배포 관문으로 묶는다.

**Tech Stack:** Rust, rusqlite, aes-gcm, rand, keyring, zeroize, Tauri store/updater/process plugins, React, TypeScript, GitHub Actions, Playwright

**Spec:** `docs/superpowers/specs/2026-08-28-smartcat-translate-design.md`

## Global Constraints

- 번역 기록 기본값은 로컬 암호화, 30일 보관이다.
- 시크릿 번역은 원문, 번역문, 파일 경로와 미리보기를 영구 저장하지 않는다.
- 인증 토큰은 Codex가 관리하며 앱 저장소에 복사하지 않는다.
- 임시 파일은 작업 완료 또는 복구 기간 종료 시 삭제한다.
- 앱 오류 로그와 개발 기록에는 번역 내용, OCR 개인정보, 인증 정보와 전체 경로가 없어야 한다.
- 업데이트는 변경 내용을 보여주고 사용자가 승인한 경우에만 설치한다.
- Windows와 Intel/Apple Silicon macOS 산출물을 GitHub Actions에서 검증한다.

---

### Task 1: OS-backed encryption key and encrypted payload envelope

**Files:**
- Create: `src-tauri/src/storage/mod.rs`
- Create: `src-tauri/src/storage/crypto.rs`
- Create: `src-tauri/src/storage/key_store.rs`
- Create: `src-tauri/tests/storage_crypto.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/lib.rs`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `KeyStore`, `OsKeyStore`, `MemoryKeyStore`, `CryptoBox::seal`, `CryptoBox::open`, `EncryptedEnvelope`

- [ ] **Step 1: Write failing authenticated-encryption tests**

```rust
#[test]
fn decrypts_only_with_matching_key_and_context() {
    let crypto = CryptoBox::from_key([7_u8; 32]);
    let envelope = crypto.seal(b"translation", b"history:42").unwrap();
    assert_eq!(crypto.open(&envelope, b"history:42").unwrap(), b"translation");
    assert!(crypto.open(&envelope, b"history:43").is_err());
}

#[test]
fn detects_ciphertext_tampering() {
    let crypto = CryptoBox::from_key([9_u8; 32]);
    let mut envelope = crypto.seal(b"private", b"job:1").unwrap();
    envelope.ciphertext[0] ^= 1;
    assert!(crypto.open(&envelope, b"job:1").is_err());
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test storage_crypto`

Expected: FAIL because storage crypto does not exist.

- [ ] **Step 2: Implement versioned AES-256-GCM envelopes**

```rust
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct EncryptedEnvelope {
    pub version: u8,
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}
```

Generate a fresh 96-bit nonce per write, bind table and row ID as associated data, and zeroize plaintext buffers and key copies after use. Reject unknown versions, nonce reuse in tests and authentication failures.

- [ ] **Step 3: Implement OS credential storage**

Use service `com.smartcat.translate` and account `local-data-key`. On first run create 32 random bytes and store Base64 in Windows Credential Manager or macOS Keychain through `keyring`. Never fall back to a plaintext file. Return `SecureStorageUnavailable` with platform instructions.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test storage_crypto`

Expected: PASS for round trip, wrong key, wrong context, tamper, version and memory-key tests.

- [ ] **Step 4: Commit encryption primitives**

```bash
git add src-tauri/src/storage src-tauri/tests/storage_crypto.rs src-tauri/Cargo.toml src-tauri/src/lib.rs PROJECT_LOG.txt
git commit -m "feat: encrypt local translation data"
```

### Task 2: Translation history, retention and secret mode

**Files:**
- Create: `src-tauri/src/storage/database.rs`
- Create: `src-tauri/src/storage/history.rs`
- Create: `src-tauri/migrations/0001_history.sql`
- Create: `src-tauri/tests/history_store.rs`
- Create: `src/features/history/historyApi.ts`
- Create: `src/features/history/HistoryView.tsx`
- Create: `src/features/history/HistoryView.test.tsx`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src/app/App.tsx`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `HistoryStore::save/list/read/delete/purge_expired`, `HistoryPolicy`, Tauri history commands

- [ ] **Step 1: Write failing retention and secret-mode tests**

```rust
#[test]
fn secret_jobs_are_never_inserted() {
    let store = test_store();
    store.save(secret_record()).unwrap();
    assert!(store.list(50, None).unwrap().is_empty());
}

#[test]
fn purge_removes_records_older_than_thirty_days() {
    let clock = FixedClock::at("2026-08-28T00:00:00Z");
    let store = test_store_with(clock);
    store.insert_at(record(), "2026-07-28T23:59:59Z").unwrap();
    assert_eq!(store.purge_expired(30).unwrap(), 1);
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test history_store`

Expected: FAIL because `HistoryStore` does not exist.

- [ ] **Step 2: Create the encrypted history schema**

```sql
CREATE TABLE history (
  id TEXT PRIMARY KEY NOT NULL,
  created_at TEXT NOT NULL,
  kind TEXT NOT NULL,
  source_language TEXT,
  target_language TEXT NOT NULL,
  source_blob BLOB NOT NULL,
  result_blob BLOB NOT NULL,
  display_name_blob BLOB,
  warning_count INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX history_created_at ON history(created_at DESC);
```

Encrypt source, result and display name separately with row/column associated data. Store no full path and no plaintext preview. Use WAL mode, foreign keys and `PRAGMA secure_delete=ON`.

- [ ] **Step 3: Implement policy and UI**

Default `enabled=true`, `retentionDays=30`. History list decrypts only the visible page. The UI supports open, copy, delete, delete all, retention selector and a persistent `시크릿 번역` switch in translation surfaces. Secret mode displays a clear non-recording indicator.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test history_store && pnpm test -- HistoryView.test.tsx`

Expected: PASS for encryption at rest, pagination, purge, delete, corrupted blob and secret mode.

- [ ] **Step 4: Commit encrypted history**

```bash
git add src-tauri/src/storage src-tauri/migrations src-tauri/tests/history_store.rs src/features/history src/app/App.tsx src-tauri/Cargo.toml PROJECT_LOG.txt
git commit -m "feat: add encrypted local translation history"
```

### Task 3: Recoverable job state machine

**Files:**
- Create: `src-tauri/src/storage/jobs.rs`
- Create: `src-tauri/migrations/0002_jobs.sql`
- Create: `src-tauri/tests/job_recovery.rs`
- Create: `src/features/history/RecoveryPrompt.tsx`
- Create: `src/features/history/RecoveryPrompt.test.tsx`
- Modify: `src-tauri/src/app_state.rs`
- Modify: `src-tauri/src/documents/pipeline.rs`
- Modify: `src-tauri/src/capture/mod.rs`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `JobStage`, `JobCheckpoint`, `JobStore::checkpoint/recoverable/resume/cancel`, `RecoveryPolicy`

- [ ] **Step 1: Write failing transition and crash-recovery tests**

```rust
#[test]
fn rejects_skipping_from_extract_to_save() {
    let mut job = job_at(JobStage::Extract);
    assert!(job.transition(JobStage::Save).is_err());
}

#[tokio::test]
async fn resumes_after_last_completed_batch() {
    let store = crashed_job_with_batches(10, 6);
    let backend = CountingBackend::default();
    resume_job(store, &backend).await.unwrap();
    assert_eq!(backend.translated_batch_indices(), vec![6, 7, 8, 9]);
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test job_recovery`

Expected: FAIL because job storage does not exist.

- [ ] **Step 2: Implement explicit allowed transitions**

Allowed path is `Queued → Extract → Translate → Rebuild → Validate → Save → Completed`; every active stage may transition to `Paused`, `Cancelled` or `Failed`; `Paused` may return only to its prior active stage. Persist stage, completed unit IDs, sanitized source fingerprint, options and encrypted resume payload.

- [ ] **Step 3: Implement recovery retention and secret behavior**

Normal jobs retain incomplete checkpoints for 7 days. Secret jobs keep checkpoints only in memory and cannot survive app restart; the UI states this before starting a secret document job. On startup, verify source fingerprint before offering resume. Cancel securely deletes checkpoint rows and partial files.

- [ ] **Step 4: Implement recovery prompt and tests**

Show filename only, kind, last stage, progress, age, `계속`, `삭제`, and `나중에`. A missing or changed source disables resume and offers deletion. Do not decrypt job content until `계속` is chosen.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test job_recovery && pnpm test -- RecoveryPrompt.test.tsx`

Expected: PASS for every transition, crash points, changed source, expired job, cancellation and secret mode.

- [ ] **Step 5: Commit recovery**

```bash
git add src-tauri/src/storage/jobs.rs src-tauri/migrations/0002_jobs.sql src-tauri/tests/job_recovery.rs src/features/history src-tauri/src/app_state.rs src-tauri/src/documents/pipeline.rs src-tauri/src/capture/mod.rs PROJECT_LOG.txt
git commit -m "feat: resume interrupted translation jobs"
```

### Task 4: Temporary-data cleanup and privacy-safe diagnostics

**Files:**
- Create: `src-tauri/src/storage/cleanup.rs`
- Create: `src-tauri/src/core/diagnostics.rs`
- Create: `src-tauri/tests/privacy_audit.rs`
- Create: `scripts/scan-sensitive-logs.mjs`
- Create: `scripts/scan-sensitive-logs.test.mjs`
- Modify: `package.json`
- Modify: `PROJECT_LOG.txt`
- Modify: `RECORDING_POLICY.txt`

**Interfaces:**
- Produces: `CleanupService::on_start/on_job_complete/purge`, `DiagnosticEvent`, `pnpm privacy:check`

- [ ] **Step 1: Write failing cleanup and log-scanner tests**

Test that completed jobs delete temporary OCR images and OOXML fragments, recoverable jobs retain only encrypted checkpoints, files older than 7 days are purged, symlinks cannot escape the app temp root, and the scanner rejects bearer tokens, email addresses, source phrases and absolute user paths.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test privacy_audit && node --test scripts/scan-sensitive-logs.test.mjs`

Expected: FAIL because cleanup and scanner do not exist.

- [ ] **Step 2: Implement root-confined cleanup**

Canonicalize the app temp root once. Before every deletion canonicalize the candidate and require it to start with that root plus a path separator. Delete only job UUID directories created by the app. Never follow symlinks. Record count and bytes only.

- [ ] **Step 3: Implement typed diagnostics and a CI scanner**

Allow fields `event`, `outcome`, `durationMs`, `jobKind`, `stage`, `errorCode`, `itemCount`, `byteCount`, `platform`, `appVersion`. Reject arbitrary maps. The scanner reads captured test logs and fails on token prefixes, emails, Windows/macOS/Linux home paths and seeded canary source strings.

Add `"privacy:check": "node scripts/scan-sensitive-logs.mjs test-results/logs"` to `package.json`.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test privacy_audit && pnpm privacy:check`

Expected: PASS on sanitized logs and FAIL in the scanner's negative fixtures.

- [ ] **Step 4: Commit privacy enforcement**

```bash
git add src-tauri/src/storage/cleanup.rs src-tauri/src/core/diagnostics.rs src-tauri/tests/privacy_audit.rs scripts package.json pnpm-lock.yaml PROJECT_LOG.txt RECORDING_POLICY.txt
git commit -m "feat: enforce private cleanup and diagnostics"
```

### Task 5: User-approved signed updates

**Files:**
- Create: `src/features/settings/UpdatePanel.tsx`
- Create: `src/features/settings/UpdatePanel.test.tsx`
- Create: `src-tauri/src/commands/update.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/capabilities/default.json`
- Modify: `PROJECT_LOG.txt`

**Interfaces:**
- Produces: `check_for_update`, `install_update`, `<UpdatePanel />`

- [ ] **Step 1: Write failing consent tests**

Test no background install, release notes shown before enablement, explicit `다운로드 및 설치`, progress, signature failure, network failure, restart confirmation and declining an update.

Run: `pnpm test -- UpdatePanel.test.tsx`

Expected: FAIL because UpdatePanel does not exist.

- [ ] **Step 2: Restrict updater endpoints and commands**

Configure one HTTPS GitHub Releases updater endpoint and one committed public update key. `check_for_update` returns version, release notes, published date and size. `install_update` requires a one-use consent token issued only after that exact version was displayed. Reject version mismatch and expired consent after 15 minutes.

- [ ] **Step 3: Implement rollback-safe user flow**

Download to updater cache, verify Tauri signature before prompting restart, retain the previous installer reference and write `last-known-good.json` only after the new version reaches the main window. On failed startup, show instructions and a button that launches the retained installer; never silently execute rollback.

Run: `pnpm test -- UpdatePanel.test.tsx && cargo test --manifest-path src-tauri/Cargo.toml commands::update`

Expected: PASS.

- [ ] **Step 4: Commit updater**

```bash
git add src/features/settings/UpdatePanel.tsx src/features/settings/UpdatePanel.test.tsx src-tauri/src/commands/update.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src-tauri/tauri.conf.json src-tauri/capabilities/default.json PROJECT_LOG.txt
git commit -m "feat: add consent-based signed updates"
```

### Task 6: Cross-platform CI, packaging and release provenance

**Files:**
- Create: `.github/workflows/release.yml`
- Create: `scripts/verify-runtime-assets.mjs`
- Create: `scripts/generate-sbom.mjs`
- Create: `docs/release-checklist.md`
- Modify: `.github/workflows/ci.yml`
- Modify: `package.json`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `PROJECT_LOG.txt`
- Modify: `CHANGELOG.md`

**Interfaces:**
- Produces: Windows MSI/EXE, macOS Intel/Apple Silicon app/DMG, checksums, SBOM, updater metadata

- [ ] **Step 1: Add failing CI configuration checks**

Create a Node test that parses both workflow YAML files and asserts CI contains frontend, Rust, records and privacy jobs; release contains `windows-latest`, `macos-15-intel` Intel and `macos-14` Apple Silicon targets; every release job runs tests before `tauri-action`.

Run: `node --test scripts/workflow-policy.test.mjs`

Expected: FAIL because release workflow does not exist.

- [ ] **Step 2: Implement the release matrix**

```yaml
strategy:
  fail-fast: false
  matrix:
    include:
      - os: windows-latest
        target: x86_64-pc-windows-msvc
        bundles: msi,nsis
      - os: macos-15-intel
        target: x86_64-apple-darwin
        bundles: app,dmg
      - os: macos-14
        target: aarch64-apple-darwin
        bundles: app,dmg
```

Every matrix leg checks out the tag, installs pinned pnpm and stable Rust, restores caches, verifies Codex/PDFium asset checksums, runs frontend/Rust/privacy tests, builds, signs, packages and uploads artifacts. Release creation occurs only after all legs succeed.

- [ ] **Step 3: Configure signing without exposing secrets**

Windows reads the certificate and password from GitHub environment secrets and signs EXE/MSI. macOS imports a temporary keychain certificate, signs nested binaries, enables hardened runtime, submits for notarization, waits for acceptance, staples app/DMG and deletes the temporary keychain. Pull-request workflows never receive signing secrets and produce unsigned test artifacts labeled `UNSIGNED`.

- [ ] **Step 4: Generate provenance and SBOM**

Produce SHA-256 sums for every artifact, Cargo and npm dependency SBOMs, bundled runtime licenses, commit SHA, workflow run ID and test summary. Attach them to the draft GitHub Release with release notes from `CHANGELOG.md`.

Run: `node --test scripts/workflow-policy.test.mjs && pnpm records:test`

Expected: PASS and workflow syntax validates with actionlint.

- [ ] **Step 5: Commit release automation**

```bash
git add .github/workflows scripts docs/release-checklist.md package.json pnpm-lock.yaml src-tauri/tauri.conf.json PROJECT_LOG.txt CHANGELOG.md
git commit -m "ci: build signed Windows and macOS releases"
```

### Task 7: Complete privacy, recovery and release acceptance gate

**Files:**
- Create: `tests/e2e/privacy-recovery.spec.ts`
- Create: `tests/e2e/update-consent.spec.ts`
- Create: `tests/release/acceptance.ps1`
- Create: `tests/release/acceptance.sh`
- Modify: `docs/release-checklist.md`
- Modify: `PROJECT_LOG.txt`
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: complete application
- Produces: documented prerelease acceptance evidence

- [ ] **Step 1: Write failing end-to-end privacy and recovery tests**

Seed canary text, translate it, crash during document batch 3, restart, resume at batch 3, delete history, purge temp data, export diagnostics and assert the canary appears only inside decrypted UI state and never in database plaintext search, logs, temp leftovers or diagnostics archive.

Run: `pnpm exec playwright test tests/e2e/privacy-recovery.spec.ts`

Expected: FAIL until all lifecycle integrations are wired.

- [ ] **Step 2: Write update-consent end-to-end tests**

Serve a locally signed fake update manifest. Assert check displays release notes, declining downloads nothing, consenting downloads and verifies, a tampered signature installs nothing, and restart requires confirmation.

Run: `pnpm exec playwright test tests/e2e/update-consent.spec.ts`

Expected: PASS after updater integration.

- [ ] **Step 3: Run the complete local gate**

```bash
pnpm records:test
pnpm test
pnpm build
pnpm privacy:check
pnpm exec playwright test
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all commands exit 0 and the workspace is clean except the intentional record updates.

- [ ] **Step 4: Run installed-app smoke tests**

On Windows, `acceptance.ps1` installs the unsigned prerelease in a disposable test account, launches, exercises mock login/text/hotkey/capture/document paths, uninstalls and verifies user documents remain. On both macOS architectures, `acceptance.sh` mounts the DMG, copies the app, verifies signature metadata, launches with mock services, exercises permissions/error states and removes only the test app data directory.

- [ ] **Step 5: Record evidence and create the prerelease commit**

Record operating systems, architectures, artifact hashes, test counts, expected unsigned warnings and remaining signing requirements in `PROJECT_LOG.txt`; finalize `CHANGELOG.md` and check every item in `docs/release-checklist.md`.

```bash
git add tests docs/release-checklist.md PROJECT_LOG.txt CHANGELOG.md
git commit -m "test: certify cross-platform prerelease"
```
