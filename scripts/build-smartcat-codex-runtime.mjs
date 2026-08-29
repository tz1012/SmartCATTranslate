import { createHash, randomUUID } from "node:crypto";
import {
  chmod,
  cp,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rename,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawn } from "node:child_process";

const EXPECTED = Object.freeze({
  repository: "https://github.com/openai/codex",
  tag: "rust-v0.144.4",
  commit: "8c68d4c87dc54d38861f5114e920c3de2efa5876",
  archiveUrl:
    "https://codeload.github.com/openai/codex/tar.gz/refs/tags/rust-v0.144.4",
  archiveSha256: "14c173d78f0c22da73e4ca1a205836b525e1dd9fe7db9b4ddea62214b2cc5009",
  patchVersion: "smartcat-1",
});
const TARGETS = Object.freeze([
  "x86_64-pc-windows-msvc",
  "x86_64-apple-darwin",
  "aarch64-apple-darwin",
]);
const MAX_ARCHIVE_BYTES = 32 * 1024 * 1024;

export async function loadAndValidatePin(pinPath) {
  const path = fileURLToPathIfNeeded(pinPath);
  let pin;
  try {
    pin = JSON.parse(await readFile(path, "utf8"));
  } catch {
    throw new Error("pin_invalid");
  }
  if (
    pin?.schemaVersion !== 1 ||
    pin?.patchVersion !== EXPECTED.patchVersion ||
    pin?.upstream?.repository !== EXPECTED.repository ||
    pin?.upstream?.tag !== EXPECTED.tag ||
    pin?.upstream?.commit !== EXPECTED.commit ||
    pin?.upstream?.archiveUrl !== EXPECTED.archiveUrl ||
    pin?.upstream?.archiveSha256 !== EXPECTED.archiveSha256 ||
    pin?.patchFile !== "smartcat.patch" ||
    !/^[0-9a-f]{64}$/.test(pin?.patchSha256 ?? "") ||
    !pin?.prePatchFiles ||
    typeof pin.prePatchFiles !== "object" ||
    Object.keys(pin.prePatchFiles).length < 1 ||
    !pin?.targets ||
    Object.keys(pin.targets).sort().join("\n") !== [...TARGETS].sort().join("\n")
  ) {
    throw new Error("pin_identity_invalid");
  }
  for (const [pathName, hash] of Object.entries(pin.prePatchFiles)) {
    requireSafeRelativePath(pathName);
    if (!/^[0-9a-f]{64}$/.test(hash)) throw new Error("prepatch_hash_invalid");
  }
  for (const target of TARGETS) {
    const expectedBinary = target.includes("windows") ? "codex.exe" : "codex";
    if (pin.targets[target]?.binary !== expectedBinary) {
      throw new Error("target_binary_invalid");
    }
  }
  return pin;
}

export async function verifyPinnedSourceTree(sourceRoot, pin) {
  const root = resolve(sourceRoot);
  for (const [relativePath, expectedHash] of Object.entries(pin.prePatchFiles)) {
    requireSafeRelativePath(relativePath);
    const path = resolve(root, relativePath);
    if (!isWithin(root, path)) throw new Error("prepatch_path_invalid");
    let metadata;
    try {
      metadata = await lstat(path);
    } catch {
      throw new Error("prepatch_file_missing");
    }
    if (!metadata.isFile() || metadata.isSymbolicLink()) {
      throw new Error("prepatch_file_invalid");
    }
    const actual = sha256(await readFile(path));
    if (actual !== expectedHash) throw new Error("prepatch_hash_mismatch");
  }
}

export function buildCargoInvocation(target) {
  if (!TARGETS.includes(target)) throw new Error("target_unsupported");
  return {
    command: "cargo",
    args: [
      "build",
      "--locked",
      "--release",
      "--package",
      "codex-cli",
      "--target",
      target,
    ],
  };
}

export function buildVerificationInvocations() {
  return [
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
  ];
}

export function buildCargoEnvironment(targetDirectory) {
  return {
    CARGO_TARGET_DIR: targetDirectory,
    CARGO_INCREMENTAL: "0",
    CARGO_PROFILE_DEV_DEBUG: "0",
    CARGO_PROFILE_TEST_DEBUG: "0",
  };
}

export async function buildRuntime({ target, outputRoot, cacheRoot } = {}) {
  if (!TARGETS.includes(target)) throw new Error("target_unsupported");
  const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const patchRoot = join(repositoryRoot, "runtime-patches", "codex-0.144.4-smartcat");
  const pin = await loadAndValidatePin(join(patchRoot, "pin.json"));
  const patchPath = join(patchRoot, pin.patchFile);
  if (sha256(await readFile(patchPath)) !== pin.patchSha256) {
    throw new Error("patch_hash_mismatch");
  }

  const cache = resolve(cacheRoot ?? join(repositoryRoot, ".runtime-cache"));
  await mkdir(cache, { recursive: true });
  const archivePath = join(cache, `${pin.upstream.tag}.tar.gz`);
  await ensurePinnedArchive(pin.upstream.archiveUrl, archivePath, pin.upstream.archiveSha256);

  const work = await mkdtemp(join(tmpdir(), "smartcat-codex-build-"));
  try {
    const archiveRoot = `codex-${pin.upstream.tag}`;
    await run("tar", [
      "-xzf",
      archivePath,
      "--exclude",
      `${archiveRoot}/codex-rs/vendor/bubblewrap/LICENSE`,
      "-C",
      work,
    ]);
    const sourceRoot = join(work, archiveRoot);
    await cp(
      join(sourceRoot, "codex-rs", "vendor", "bubblewrap", "COPYING"),
      join(sourceRoot, "codex-rs", "vendor", "bubblewrap", "LICENSE"),
      { force: false },
    );
    await verifyPinnedSourceTree(sourceRoot, pin);
    await run("git", ["apply", "--check", "--whitespace=error-all", patchPath], sourceRoot);
    await run("git", ["apply", "--whitespace=error-all", patchPath], sourceRoot);

    const cargoTargetDirectory = resolve(
      process.env.SMARTCAT_CODEX_CARGO_TARGET_DIR ?? join(cache, "cargo-target", target),
    );
    const cargoEnvironment = buildCargoEnvironment(cargoTargetDirectory);
    for (const verification of buildVerificationInvocations()) {
      await run(
        verification.command,
        verification.args,
        join(sourceRoot, "codex-rs"),
        cargoEnvironment,
      );
    }
    const invocation = buildCargoInvocation(target);
    await run(
      invocation.command,
      invocation.args,
      join(sourceRoot, "codex-rs"),
      cargoEnvironment,
    );
    const sourceBinary = join(
      cargoTargetDirectory,
      target,
      "release",
      pin.targets[target].binary,
    );
    const outputDirectory = resolve(outputRoot ?? join(repositoryRoot, "src-tauri", "binaries"));
    await mkdir(outputDirectory, { recursive: true });
    const extension = target.includes("windows") ? ".exe" : "";
    const outputBinary = join(outputDirectory, `smartcat-codex-${target}${extension}`);
    const staging = `${outputBinary}.tmp-${process.pid}`;
    await cp(sourceBinary, staging, { force: false });
    if (!target.includes("windows")) await chmod(staging, 0o755);
    await rename(staging, outputBinary);

    const metadata = await stat(outputBinary);
    const manifest = {
      schemaVersion: 1,
      target,
      binary: basename(outputBinary),
      sha256: sha256(await readFile(outputBinary)),
      size: metadata.size,
      upstreamTag: pin.upstream.tag,
      upstreamCommit: pin.upstream.commit,
      sourceArchiveSha256: pin.upstream.archiveSha256,
      patchVersion: pin.patchVersion,
      patchSha256: pin.patchSha256,
      cargoLocked: true,
    };
    const manifestPath = join(outputDirectory, `smartcat-codex-${target}.manifest.json`);
    await atomicWrite(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
    const provenancePath = join(outputDirectory, `smartcat-codex-${target}.provenance.json`);
    await atomicWrite(
      provenancePath,
      `${JSON.stringify(
        {
          schemaVersion: 1,
          builder: "scripts/build-smartcat-codex-runtime.mjs",
          invocation: { target, cargoLocked: true, profile: "release" },
          materials: [
            { uri: pin.upstream.repository, digest: { gitCommit: pin.upstream.commit } },
            { uri: pin.upstream.archiveUrl, digest: { sha256: pin.upstream.archiveSha256 } },
            { uri: pin.patchFile, digest: { sha256: pin.patchSha256 } },
          ],
          subject: [{ name: basename(outputBinary), digest: { sha256: manifest.sha256 } }],
          reproducibilityClaim: "reproducible-inputs-only",
        },
        null,
        2,
      )}\n`,
    );
    const sbomPath = join(outputDirectory, `smartcat-codex-${target}.sbom.cdx.json`);
    await atomicWrite(
      sbomPath,
      `${JSON.stringify(
        {
          bomFormat: "CycloneDX",
          specVersion: "1.6",
          serialNumber: `urn:uuid:${randomUUID()}`,
          version: 1,
          metadata: { component: { type: "application", name: "smartcat-codex", version: "0.144.4-smartcat.1" } },
          components: [
            {
              type: "application",
              name: "smartcat-codex",
              version: "0.144.4-smartcat.1",
              hashes: [{ alg: "SHA-256", content: manifest.sha256 }],
              licenses: [{ license: { id: "Apache-2.0" } }],
              properties: [
                { name: "smartcat:upstreamCommit", value: pin.upstream.commit },
                { name: "smartcat:patchSha256", value: pin.patchSha256 },
              ],
            },
          ],
        },
        null,
        2,
      )}\n`,
    );
    if (!outputRoot) {
      await atomicWrite(
        join(repositoryRoot, "src-tauri", "resources", "smartcat-codex-runtime.json"),
        `${JSON.stringify(manifest, null, 2)}\n`,
        true,
      );
    }
    return { outputBinary, manifestPath, provenancePath, sbomPath, manifest };
  } finally {
    await rm(work, { recursive: true, force: true });
  }
}

async function ensurePinnedArchive(url, archivePath, expectedHash) {
  try {
    const existing = await readFile(archivePath);
    if (sha256(existing) === expectedHash) return;
  } catch {}
  const response = await fetch(url, { redirect: "error", signal: AbortSignal.timeout(60_000) });
  if (!response.ok || !response.body) throw new Error("source_download_failed");
  const contentLength = validateOptionalContentLength(response.headers.get("content-length"));
  const chunks = [];
  let total = 0;
  for await (const chunk of response.body) {
    total += chunk.byteLength;
    if (total > MAX_ARCHIVE_BYTES) throw new Error("source_too_large");
    chunks.push(Buffer.from(chunk));
  }
  const bytes = Buffer.concat(chunks);
  if ((contentLength !== null && bytes.length !== contentLength) || sha256(bytes) !== expectedHash) {
    throw new Error("source_hash_mismatch");
  }
  await atomicWrite(archivePath, bytes);
}

export function validateOptionalContentLength(value) {
  if (value === null) return null;
  const length = Number(value);
  if (!Number.isSafeInteger(length) || length < 1 || length > MAX_ARCHIVE_BYTES) {
    throw new Error("source_size_invalid");
  }
  return length;
}

async function atomicWrite(path, contents, _replace = false) {
  await mkdir(dirname(path), { recursive: true });
  const temporary = `${path}.tmp-${process.pid}`;
  await writeFile(temporary, contents, { flag: "wx", mode: 0o600 });
  await rename(temporary, path);
}

function requireSafeRelativePath(pathName) {
  if (
    typeof pathName !== "string" ||
    pathName.length < 1 ||
    isAbsolute(pathName) ||
    pathName.includes("\\") ||
    pathName.split("/").some((part) => part === "" || part === "." || part === "..")
  ) {
    throw new Error("prepatch_path_invalid");
  }
}

function isWithin(root, path) {
  const rel = relative(root, path);
  return rel !== ".." && !rel.startsWith(`..${sep}`) && !isAbsolute(rel);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function fileURLToPathIfNeeded(value) {
  return value instanceof URL ? fileURLToPath(value) : resolve(value);
}

function run(command, args, cwd, environment = {}) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, {
      cwd,
      stdio: "inherit",
      windowsHide: true,
      shell: false,
      env: { ...process.env, ...environment },
    });
    child.on("error", () => reject(new Error("build_command_failed")));
    child.on("exit", (code) =>
      code === 0 ? resolvePromise() : reject(new Error("build_command_failed")),
    );
  });
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const targetIndex = process.argv.indexOf("--target");
  const target = targetIndex >= 0 ? process.argv[targetIndex + 1] : undefined;
  buildRuntime({ target }).catch((error) => {
    process.stderr.write(`build-smartcat-codex-runtime: ${error?.message ?? "failed"}\n`);
    process.exitCode = 1;
  });
}
