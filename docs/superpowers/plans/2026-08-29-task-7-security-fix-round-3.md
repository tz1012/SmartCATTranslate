# Task 7 Security Fix Round 3 Implementation Plan

> **Execution rule:** Apply strict test-driven development: add each boundary test, capture its expected failure, then make the smallest production change that turns it green.

**Goal:** Replace the unusable Windows AppContainer launch with an audited, application-bundled SmartCAT downstream build of Codex `rust-v0.144.4` that can authenticate normally but can never advertise model tools or discover user/project instructions.

**Architecture:** Reproducible-input scripts fetch and verify the pinned upstream archive, verify every patched file's original hash, apply a small unified patch, and build with the upstream lockfile. Tauri bundles the per-target patched `codex` sidecar and a build-generated hash/provenance manifest. Startup verifies the sidecar bytes, initializes one live JSONL session, and requires a patch-only attestation response on that same session before installing account or translation services. The downstream Codex patch clears tools at the final Responses request assembly boundary and short-circuits all user/project instruction loading.

**Pinned upstream:** tag `rust-v0.144.4`, commit `8c68d4c87dc54d38861f5114e920c3de2efa5876`, codeload archive SHA-256 `14c173d78f0c22da73e4ca1a205836b525e1dd9fe7db9b4ddea62214b2cc5009`.

---

### Task 1: Supply-chain pin and patch harness

**Files:**
- Create: `runtime-patches/codex-0.144.4-smartcat/*`
- Create: `scripts/build-smartcat-codex-runtime.mjs`
- Create: `scripts/build-smartcat-codex-runtime.test.mjs`

1. Add tests for exact pin parsing, archive/hash drift, pre-patch file drift, path traversal, target allowlisting, patch application, and locked build command construction.
2. Run the focused Node test and record RED.
3. Implement the fail-closed verifier/builder and provenance/SBOM generator.
4. Run focused tests and record GREEN.

### Task 2: Downstream Codex invariants and attestation

**Upstream patch files (expected):**
- Modify: `codex-rs/core/src/client.rs`
- Modify: `codex-rs/core/src/client_tests.rs`
- Modify: `codex-rs/core/src/agents_md.rs`
- Modify: `codex-rs/core/src/agents_md_tests.rs`
- Modify: `codex-rs/app-server-protocol/src/protocol/common.rs`
- Modify: `codex-rs/app-server-protocol/src/protocol/v2/mod.rs`
- Create: `codex-rs/app-server-protocol/src/protocol/v2/smartcat.rs`
- Modify: `codex-rs/app-server/src/message_processor.rs`
- Modify/add: focused App Server integration tests

1. Add a final-request-boundary test proving a deliberately populated prompt yields zero HTTP and Responses-Lite tool declarations.
2. Add an instruction test with seeded global and ancestor `AGENTS.md` proving no text is loaded and no sources are reported by `thread/start`.
3. Add a protocol/integration test proving `smartcat/attestation` is unavailable before initialize and returns exact commit, patch version, `toolCount: 0`, and `instructionDiscovery: false` after initialize.
4. Run focused upstream tests and record RED.
5. Implement the smallest unconditional downstream changes and run the same tests GREEN.
6. Export one reviewable patch and bind its pre-patch hashes in the pin file.

### Task 3: Embedded sidecar trust and live-session attestation

**Files:**
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/build.rs`
- Modify: `src-tauri/src/codex/{manifest,install,process,runtime,bootstrap}.rs`
- Modify: `src-tauri/tests/{codex_bootstrap,codex_runtime}.rs`
- Create/update: bundled provenance, `PATCH-NOTICE`, SBOM, LICENSE/NOTICE resources

1. Add SmartCAT tests that reject a stock/fake runtime without exact attestation, reject a changed embedded hash, and accept a fake live session with exact attestation.
2. Run focused Rust tests and record RED.
3. Replace download/system resolution with the verified per-target embedded sidecar; remove the AppContainer production path with no unsandboxed/system fallback.
4. Require initialize then attestation on the same owned transport before account service installation.
5. Run focused Rust tests GREEN, including fake account-read and malicious translation prompt boundaries without model network calls.

### Task 4: Three-target CI and release provenance

**Files:**
- Modify: `.github/workflows/ci.yml`
- Create: explicit release workflow if needed
- Modify: package scripts and records

1. Add static workflow tests/checks for Windows x86_64, macOS x86_64, and macOS aarch64; release upload must be explicit only.
2. Build/test the patched runtime, emit SHA/provenance/SBOM, build/test Tauri, and request GitHub artifact attestations/checksums.
3. Do not publish artifacts from ordinary CI or this task.

### Task 5: Verification, records, and commit

1. Locally build the patched Windows runtime with `--locked`.
2. Run downstream tool-free, instruction-discovery, and attestation tests.
3. Run SmartCAT real initialize/account fake-boundary/translation malicious-prompt tests; prove stock Codex rejection.
4. Run full Rust, frontend, build, format, lint, record, pinning, source-integrity, and diff-range gates.
5. Update `DECISIONS.txt`, `PROJECT_LOG.txt`, and the Task 7 report with exact evidence, limitations, and provenance.
6. Commit the round-3 security fix and report the commit and any precise blocker.
