import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  buildCargoInvocation,
  buildCargoEnvironment,
  buildVerificationInvocations,
  loadAndValidatePin,
  validateOptionalContentLength,
  verifyPinnedSourceTree,
} from "./build-smartcat-codex-runtime.mjs";

const EXPECTED_COMMIT = "8c68d4c87dc54d38861f5114e920c3de2efa5876";
const EXPECTED_ARCHIVE_SHA = "14c173d78f0c22da73e4ca1a205836b525e1dd9fe7db9b4ddea62214b2cc5009";

test("pin accepts only exact upstream identity and three sidecar targets", async () => {
  const pin = await loadAndValidatePin(
    new URL("../runtime-patches/codex-0.144.4-smartcat/pin.json", import.meta.url),
  );

  assert.equal(pin.upstream.tag, "rust-v0.144.4");
  assert.equal(pin.upstream.commit, EXPECTED_COMMIT);
  assert.equal(pin.upstream.archiveSha256, EXPECTED_ARCHIVE_SHA);
  assert.deepEqual(Object.keys(pin.targets).sort(), [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
  ]);
  assert.equal(pin.patchVersion, "smartcat-1");
  assert.ok(Object.keys(pin.prePatchFiles).length >= 8);
});

test("pin rejects a source URL or hash that drifts from the audited upstream", async () => {
  const directory = await mkdtemp(join(tmpdir(), "smartcat-pin-"));
  const path = join(directory, "pin.json");
  const fixture = basePin();
  fixture.upstream.archiveUrl = "https://example.invalid/source.tar.gz";
  await writeFile(path, JSON.stringify(fixture));

  await assert.rejects(loadAndValidatePin(path), /pin_identity_invalid/);

  fixture.upstream.archiveUrl =
    "https://codeload.github.com/openai/codex/tar.gz/refs/tags/rust-v0.144.4";
  fixture.upstream.archiveSha256 = "a".repeat(64);
  await writeFile(path, JSON.stringify(fixture));
  await assert.rejects(loadAndValidatePin(path), /pin_identity_invalid/);
});

test("source verification fails closed on pre-patch drift and traversal", async () => {
  const directory = await mkdtemp(join(tmpdir(), "smartcat-source-"));
  await mkdir(join(directory, "codex-rs", "core", "src"), { recursive: true });
  await writeFile(join(directory, "codex-rs", "core", "src", "client.rs"), "original");
  const pin = basePin();
  pin.prePatchFiles = {
    "codex-rs/core/src/client.rs":
      "0682c5f2076f099c34cfdd15a9e063849ed437a49677e6fcc5b4198c76575be5",
  };
  await verifyPinnedSourceTree(directory, pin);

  await writeFile(join(directory, "codex-rs", "core", "src", "client.rs"), "drift");
  await assert.rejects(verifyPinnedSourceTree(directory, pin), /prepatch_hash_mismatch/);

  pin.prePatchFiles = { "../secret": "0".repeat(64) };
  await assert.rejects(verifyPinnedSourceTree(directory, pin), /prepatch_path_invalid/);
});

test("cargo invocation is locked, release-only, package-scoped, and target allowlisted", () => {
  assert.deepEqual(buildCargoInvocation("x86_64-pc-windows-msvc"), {
    command: "cargo",
    args: [
      "build",
      "--locked",
      "--release",
      "--package",
      "codex-cli",
      "--target",
      "x86_64-pc-windows-msvc",
    ],
  });
  assert.throws(() => buildCargoInvocation("x86_64-unknown-linux-gnu"), /target_unsupported/);
});

test("patched runtime build runs the three audited upstream security tests", () => {
  assert.deepEqual(buildVerificationInvocations(), [
    {
      command: "cargo",
      args: [
        "test",
        "--locked",
        "-p",
        "codex-core",
        "--lib",
        "smartcat_final_request_boundary_never_declares_tools",
      ],
    },
    {
      command: "cargo",
      args: [
        "test",
        "--locked",
        "-p",
        "codex-core",
        "--lib",
        "smartcat_runtime_ignores_user_and_ancestor_instruction_files",
      ],
    },
    {
      command: "cargo",
      args: [
        "test",
        "--locked",
        "-p",
        "codex-app-server",
        "smartcat_attests_tool_free_runtime_and_reports_no_instruction_sources",
      ],
    },
  ]);
});

test("source streaming remains bounded when codeload omits content-length", () => {
  assert.equal(validateOptionalContentLength(null), null);
  assert.equal(validateOptionalContentLength("9541346"), 9_541_346);
  assert.throws(() => validateOptionalContentLength("33554433"), /source_size_invalid/);
  assert.throws(() => validateOptionalContentLength("invalid"), /source_size_invalid/);
});

test("upstream verification disables incremental and debug-symbol disk amplification", () => {
  assert.deepEqual(buildCargoEnvironment("X:/target"), {
    CARGO_TARGET_DIR: "X:/target",
    CARGO_INCREMENTAL: "0",
    CARGO_PROFILE_DEV_DEBUG: "0",
    CARGO_PROFILE_TEST_DEBUG: "0",
  });
});

test("tracked patch notice identifies downstream limits without reproducibility overclaim", async () => {
  const notice = await readFile(
    new URL("../runtime-patches/codex-0.144.4-smartcat/PATCH-NOTICE.txt", import.meta.url),
    "utf8",
  );
  assert.match(notice, /downstream/i);
  assert.match(notice, /reproducible inputs/i);
  assert.doesNotMatch(notice, /byte-for-byte reproducible/i);
});

test("release workflow builds and attests all three sidecar targets only on explicit dispatch", async () => {
  const workflow = await readFile(
    new URL("../.github/workflows/smartcat-runtime-release.yml", import.meta.url),
    "utf8",
  );

  assert.match(workflow, /workflow_dispatch:/);
  assert.doesNotMatch(workflow, /^\s+(push|pull_request|schedule):/m);
  for (const target of [
    "x86_64-pc-windows-msvc",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
  ]) {
    assert.match(workflow, new RegExp(`target: ${target}`));
  }
  assert.match(
    workflow,
    /os: macos-15-intel\s+target: x86_64-apple-darwin/,
    "the Intel build must use GitHub's supported Intel runner label",
  );
  assert.match(
    workflow,
    /os: macos-15\s+target: aarch64-apple-darwin/,
    "the Apple Silicon build must use a supported arm64 runner label",
  );
  assert.match(workflow, /runtime:build -- --target/);
  assert.match(workflow, /actual_patched_sidecar_initializes_and_attests_on_the_live_session/);
  assert.match(workflow, /anchore\/sbom-action/);
  assert.match(workflow, /actions\/attest-build-provenance/);
  assert.match(workflow, /tauri build --config src-tauri\/tauri\.runtime\.conf\.json/);
});

test("runtime bundle overlay contains only the patched sidecar and tracked notices", async () => {
  const config = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.runtime.conf.json", import.meta.url), "utf8"),
  );

  assert.deepEqual(config.bundle.externalBin, ["binaries/smartcat-codex"]);
  assert.deepEqual(config.bundle.resources, [
    "resources/smartcat-codex-runtime.json",
    "resources/LICENSE",
    "resources/NOTICE",
    "../runtime-patches/codex-0.144.4-smartcat/PATCH-NOTICE.txt",
  ]);
});

function basePin() {
  return {
    schemaVersion: 1,
    patchVersion: "smartcat-1",
    upstream: {
      repository: "https://github.com/openai/codex",
      tag: "rust-v0.144.4",
      commit: EXPECTED_COMMIT,
      archiveUrl:
        "https://codeload.github.com/openai/codex/tar.gz/refs/tags/rust-v0.144.4",
      archiveSha256: EXPECTED_ARCHIVE_SHA,
    },
    patchFile: "smartcat.patch",
    prePatchFiles: {},
    targets: {
      "x86_64-pc-windows-msvc": { binary: "codex.exe" },
      "x86_64-apple-darwin": { binary: "codex" },
      "aarch64-apple-darwin": { binary: "codex" },
    },
  };
}
