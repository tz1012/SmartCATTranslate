import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  buildCargoCycloneDxInvocation,
  createArtifactEvidence,
  generateCargoDependencySbom,
  validateCycloneDxDocument,
  verifyArtifactChecksums,
} from "./release-evidence.mjs";

test("cargo-cyclonedx invocation covers all locked dependencies for the selected target", () => {
  assert.deepEqual(
    buildCargoCycloneDxInvocation({
      manifestPath: "src-tauri/Cargo.toml",
      target: "aarch64-apple-darwin",
      outputStem: "smartcat-app-rust-aarch64-apple-darwin.dependencies.cdx",
    }),
    {
      command: "cargo",
      args: [
        "cyclonedx",
        "--manifest-path",
        "src-tauri/Cargo.toml",
        "--format",
        "json",
        "--spec-version",
        "1.5",
        "--all",
        "--target",
        "aarch64-apple-darwin",
        "--override-filename",
        "smartcat-app-rust-aarch64-apple-darwin.dependencies.cdx",
      ],
    },
  );
  assert.throws(
    () => buildCargoCycloneDxInvocation({ manifestPath: "src-tauri/Cargo.toml", target: "linux", outputStem: "bom" }),
    /target_unsupported/,
  );
});

test("cargo dependency SBOM generation rejects a changed lockfile", async () => {
  const root = await mkdtemp(join(tmpdir(), "smartcat-cargo-sbom-"));
  const manifestPath = join(root, "Cargo.toml");
  const lockPath = join(root, "Cargo.lock");
  await writeFile(manifestPath, "[package]\nname='fixture'\nversion='0.1.0'\n");
  await writeFile(lockPath, "version = 4\n");

  await assert.rejects(
    generateCargoDependencySbom({
      manifestPath,
      lockPath,
      target: "x86_64-pc-windows-msvc",
      outputStem: "fixture.dependencies.cdx",
      destinationPath: join(root, "evidence", "fixture.cdx.json"),
      commandRunner: async () => {
        await writeFile(lockPath, "version = 4\n# drift\n");
        await writeFile(
          join(root, "fixture.dependencies.cdx.json"),
          JSON.stringify({
            bomFormat: "CycloneDX",
            specVersion: "1.5",
            version: 1,
            components: [{ type: "application", name: "fixture" }, { type: "library", name: "dep" }],
            dependencies: [{ ref: "fixture", dependsOn: ["dep"] }],
          }),
        );
      },
    }),
    /cargo_lock_changed/,
  );
});

test("cargo dependency SBOM generation validates, copies, and removes its source-side output", async () => {
  const root = await mkdtemp(join(tmpdir(), "smartcat-cargo-sbom-success-"));
  const manifestPath = join(root, "Cargo.toml");
  const lockPath = join(root, "Cargo.lock");
  const generatedPath = join(root, "fixture.dependencies.cdx.json");
  const destinationPath = join(root, "evidence", "fixture.cdx.json");
  await writeFile(manifestPath, "[package]\nname='fixture'\nversion='0.1.0'\n");
  await writeFile(lockPath, "version = 4\n");
  const bom = {
    bomFormat: "CycloneDX",
    specVersion: "1.5",
    version: 1,
    components: [{ type: "application", name: "fixture" }, { type: "library", name: "dep" }],
    dependencies: [{ ref: "fixture", dependsOn: ["dep"] }],
  };

  await generateCargoDependencySbom({
    manifestPath,
    lockPath,
    target: "x86_64-pc-windows-msvc",
    outputStem: "fixture.dependencies.cdx",
    destinationPath,
    commandRunner: async () => writeFile(generatedPath, JSON.stringify(bom)),
  });

  assert.deepEqual(JSON.parse(await readFile(destinationPath, "utf8")), bom);
  await assert.rejects(readFile(generatedPath), /ENOENT/);
});

test("artifact evidence covers the sidecar and every nonempty installer with literal hashes", async () => {
  const root = await mkdtemp(join(tmpdir(), "smartcat-release-evidence-"));
  const sidecar = join(root, "src-tauri", "binaries", "smartcat-codex-x86_64-pc-windows-msvc.exe");
  const bundle = join(root, "src-tauri", "target", "x86_64-pc-windows-msvc", "release", "bundle");
  const installer = join(bundle, "msi", "SmartCAT_0.1.0_x64_en-US.msi");
  await mkdir(join(root, "src-tauri", "binaries"), { recursive: true });
  await mkdir(join(bundle, "msi"), { recursive: true });
  await writeFile(sidecar, "patched-sidecar");
  await writeFile(installer, "signed-ready-installer");

  const result = await createArtifactEvidence({
    repositoryRoot: root,
    target: "x86_64-pc-windows-msvc",
    sidecarPath: sidecar,
    bundleRoot: bundle,
    outputRoot: join(root, "artifacts"),
  });
  const checksums = await readFile(result.checksumPath, "utf8");
  const bom = JSON.parse(await readFile(result.sbomPath, "utf8"));

  assert.equal(checksums, [
    "e6e320523309f0dd92c5355c09cfd5646494145fd0cc52ee3d426aeca6a6c880  src-tauri/binaries/smartcat-codex-x86_64-pc-windows-msvc.exe",
    "53a734d3cc63ee7bd20ef506fb3ce5972d48bbbb83a9d49fe83a0ff2a1c326f7  src-tauri/target/x86_64-pc-windows-msvc/release/bundle/msi/SmartCAT_0.1.0_x64_en-US.msi",
    "",
  ].join("\n"));
  assert.deepEqual(
    bom.components.map(({ name, hashes }) => ({ name, hashes })),
    [
      {
        name: "src-tauri/binaries/smartcat-codex-x86_64-pc-windows-msvc.exe",
        hashes: [{ alg: "SHA-256", content: "e6e320523309f0dd92c5355c09cfd5646494145fd0cc52ee3d426aeca6a6c880" }],
      },
      {
        name: "src-tauri/target/x86_64-pc-windows-msvc/release/bundle/msi/SmartCAT_0.1.0_x64_en-US.msi",
        hashes: [{ alg: "SHA-256", content: "53a734d3cc63ee7bd20ef506fb3ce5972d48bbbb83a9d49fe83a0ff2a1c326f7" }],
      },
    ],
  );
});

test("artifact evidence fails closed when an installer is missing, empty, or a link", async () => {
  const root = await mkdtemp(join(tmpdir(), "smartcat-release-invalid-"));
  const sidecar = join(root, "smartcat-codex-x86_64-pc-windows-msvc.exe");
  const bundle = join(root, "bundle");
  await mkdir(bundle);
  await writeFile(sidecar, "patched-sidecar");
  const input = {
    repositoryRoot: root,
    target: "x86_64-pc-windows-msvc",
    sidecarPath: sidecar,
    bundleRoot: bundle,
    outputRoot: join(root, "artifacts"),
  };

  await assert.rejects(createArtifactEvidence(input), /installer_missing/);
  await mkdir(join(bundle, "msi"));
  const installer = join(bundle, "msi", "SmartCAT.msi");
  await writeFile(installer, "");
  await assert.rejects(createArtifactEvidence(input), /artifact_empty/);
  await writeFile(installer, "installer");
  const linked = join(bundle, "msi", "Linked.msi");
  try {
    await symlink(installer, linked, "file");
  } catch (error) {
    if (process.platform === "win32" && error?.code === "EPERM") return;
    throw error;
  }
  await assert.rejects(createArtifactEvidence(input), /artifact_link/);
});

test("artifact checksum verification rejects a sidecar or installer changed after discovery", async () => {
  const root = await mkdtemp(join(tmpdir(), "smartcat-release-tamper-"));
  const sidecar = join(root, "src-tauri", "binaries", "smartcat-codex-x86_64-pc-windows-msvc.exe");
  const bundle = join(root, "src-tauri", "target", "x86_64-pc-windows-msvc", "release", "bundle");
  const installer = join(bundle, "msi", "SmartCAT.msi");
  await mkdir(join(root, "src-tauri", "binaries"), { recursive: true });
  await mkdir(join(bundle, "msi"), { recursive: true });
  await writeFile(sidecar, "patched-sidecar");
  await writeFile(installer, "installer-before-verification");
  const evidence = await createArtifactEvidence({
    repositoryRoot: root,
    target: "x86_64-pc-windows-msvc",
    sidecarPath: sidecar,
    bundleRoot: bundle,
    outputRoot: join(root, "artifacts"),
  });
  await writeFile(installer, "tampered-installer");

  await assert.rejects(
    verifyArtifactChecksums({ repositoryRoot: root, checksumPath: evidence.checksumPath }),
    /artifact_checksum_mismatch/,
  );
});

test("CycloneDX validation rejects empty, one-component, and dependency-free claims", () => {
  const base = { bomFormat: "CycloneDX", specVersion: "1.5", version: 1 };

  assert.throws(() => validateCycloneDxDocument(base, { minimumComponents: 2 }), /sbom_components_missing/);
  assert.throws(
    () => validateCycloneDxDocument({ ...base, components: [{ type: "library", name: "only" }] }, { minimumComponents: 2 }),
    /sbom_components_insufficient/,
  );
  assert.throws(
    () => validateCycloneDxDocument({ ...base, components: [{ type: "library", name: "a" }, { type: "library", name: "b" }] }, { minimumComponents: 2, requireDependencyGraph: true }),
    /sbom_dependency_graph_missing/,
  );
});

test("CycloneDX validation accepts a multi-component locked dependency graph", () => {
  const document = {
    bomFormat: "CycloneDX",
    specVersion: "1.5",
    version: 1,
    components: [
      { type: "application", name: "smartcat" },
      { type: "library", name: "serde" },
    ],
    dependencies: [
      { ref: "pkg:cargo/smartcat@0.1.0", dependsOn: ["pkg:cargo/serde@1.0.0"] },
      { ref: "pkg:cargo/serde@1.0.0", dependsOn: [] },
    ],
  };

  assert.doesNotThrow(() => validateCycloneDxDocument(document, { minimumComponents: 2, requireDependencyGraph: true }));
});
