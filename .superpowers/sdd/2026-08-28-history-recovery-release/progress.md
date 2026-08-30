# SDD ledger — plan: docs/superpowers/plans/2026-08-28-history-recovery-release.md

## Preflight interface scan

| Tasks | Producer → consumer | Finding / ruling |
|---|---|---|
| 1 → 2 | `CryptoBox`/`KeyStore` → encrypted history | Compatible; history must bind row+column associated data. |
| 1 → 3 | OS key + envelope → encrypted checkpoints | Compatible; secret checkpoints remain memory-only. |
| 2 ↔ 3 | SQLite migrations/history policy ↔ job state | Share one database bootstrap and migration runner; serialize schema setup. |
| 3 → documents/capture | recovery checkpoints → stage callbacks | Existing PDF fix adds deterministic unit/stage callback hooks; JobStore supplies persistence next. |
| 3 → 4 | job lifecycle → temp cleanup | Completed/cancelled/expired state drives root-confined cleanup. |
| 2 ↔ UI | history policy/secret mode → all translation surfaces | One persistent policy source; secret state must reach text, popup, capture, and document requests. |
| 4 → 6/7 | privacy scanner/diagnostics → CI/acceptance | Scanner becomes a short policy gate; no source text/full paths in fixtures or logs. |
| 5 → 6 | updater config/signature → signed artifacts | Public key/endpoint are committed only when real values exist; secrets stay in GitHub environments. |
| 6 → 7 | release matrix/artifacts → acceptance evidence | Local Windows unsigned smoke can be run; both macOS architectures require CI hardware. |

## Rulings

- Ruling: The user's latest speed instruction overrides plan steps that require creating or running new failing, failure-injection, long full-regression, or exhaustive crash-point tests. Preserve existing tests; use direct happy-path implementation, formatting, frontend build, one incremental `cargo check --lib`, records/privacy policy checks, and short smoke only — this reduces proof depth but avoids hours of negative-path execution.
- Ruling: Batch Tasks 1–4 into one implementation milestone because encryption, database bootstrap, job checkpoints, cleanup, and secret-mode propagation share files and interfaces — this reduces dispatch overhead but enlarges the review diff.
- Ruling: Authentication tokens remain exclusively Codex-managed; history/recovery stores never copy them — if the runtime contract changes, account reconnection is required rather than insecure fallback.
- Ruling: Durable document recovery is completed with Task 3 after the PDF pipeline exposes checkpoint hooks; the PDF task may not invent a plaintext interim store — until Task 3 lands, restart recovery is not yet release-ready.

Task 1–4: fix round 1/5 (crash checkpoints/temp roots addressed; retention/key/secret/terminal/cleanup findings open; commits 64e9f66..13c647d)
Task 1–4: fix round 2/5 (six open items addressed; cleanup state/order atomicity findings open; commits 13c647d..abef213)
Task 1–4: fix round 3/5 (Save-only completion addressed; cleanup metadata concurrency/recovery open; commits abef213..efb41e2)
Task 1–4: fix round 4/5 (root-shared serialization and startup UUID union addressed; commit 71b24b9; review clean)
Task 1–4: complete (commits 64e9f66..71b24b9, static review clean)
Task 1–4: verification gap — frontend build, formatting, privacy and records gates passed; Rust product compilation repeatedly progressed into the product crate but did not finish within the user-directed 60/120-second ceilings. Do not claim Rust PASS until a later packaging gate completes it.

Tasks 5–7: publication fix round 1 — retired Intel runner, setup-time LKG, unverified tag/SHA inputs, non-executed platform trust checks, flattened macOS app artifacts, metadata-only license evidence, and unpaired updater keys were ruled publication blockers. Implementation now uses macOS 15 Intel, exact refs/tags plus four-way version gate before protected secrets, pending-update/healthy handshake and backend restart consent, CI-only isolated install/launch acceptance, fail-closed Authenticode/codesign/spctl/stapler checks, preserved app archive/updater artifacts, actual dependency license texts/SBOM expressions, and updater signing canary verification. Per the user's latest speed instruction, new negative/E2E tests requested by the original plan were not added; this remains a verification gap rather than an implied pass.
