import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFile,
  lstat,
  mkdir,
  readFile,
  readdir,
  rename,
  unlink,
  writeFile,
} from "node:fs/promises";
import { dirname, extname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { pathToFileURL } from "node:url";

const TARGETS = new Set([
  "x86_64-pc-windows-msvc",
  "x86_64-apple-darwin",
  "aarch64-apple-darwin",
]);

export function buildCargoCycloneDxInvocation({ manifestPath, target, outputStem }) {
  requireTarget(target);
  if (typeof manifestPath !== "string" || manifestPath.length < 1) {
    throw new Error("manifest_path_invalid");
  }
  if (!/^[a-zA-Z0-9_.-]+$/.test(outputStem ?? "")) {
    throw new Error("sbom_output_invalid");
  }
  return {
    command: "cargo",
    args: [
      "cyclonedx",
      "--manifest-path",
      manifestPath,
      "--format",
      "json",
      "--spec-version",
      "1.5",
      "--all",
      "--target",
      target,
      "--override-filename",
      outputStem,
    ],
  };
}

export async function generateCargoDependencySbom({
  manifestPath,
  lockPath,
  target,
  outputStem,
  destinationPath,
  commandRunner = run,
}) {
  const manifest = resolve(manifestPath);
  const lock = resolve(lockPath);
  const destination = resolve(destinationPath);
  await requireRegularFile(manifest, "manifest_invalid");
  await requireRegularFile(lock, "cargo_lock_invalid");
  const before = sha256(await readFile(lock));
  const invocation = buildCargoCycloneDxInvocation({
    manifestPath: manifest,
    target,
    outputStem,
  });
  await commandRunner(
    "cargo",
    ["fetch", "--locked", "--manifest-path", manifest],
    dirname(manifest),
    {},
  );
  if (sha256(await readFile(lock)) !== before) throw new Error("cargo_lock_changed");
  await commandRunner(invocation.command, invocation.args, dirname(manifest), {
    CARGO_NET_OFFLINE: "true",
  });
  const generated = join(dirname(manifest), `${outputStem}.json`);
  try {
    if (sha256(await readFile(lock)) !== before) throw new Error("cargo_lock_changed");
    await requireRegularFile(generated, "sbom_output_invalid");
    const document = JSON.parse(await readFile(generated, "utf8"));
    validateCycloneDxDocument(document, {
      minimumComponents: 2,
      requireDependencyGraph: true,
    });
    await atomicCopy(generated, destination);
    return destination;
  } finally {
    await unlink(generated).catch((error) => {
      if (error?.code !== "ENOENT") throw error;
    });
  }
}

export async function createArtifactEvidence({
  repositoryRoot,
  target,
  sidecarPath,
  bundleRoot,
  outputRoot,
}) {
  requireTarget(target);
  const root = resolve(repositoryRoot);
  const sidecar = resolve(sidecarPath);
  const bundle = resolve(bundleRoot);
  const output = resolve(outputRoot);
  await requireDirectory(root, "repository_root_invalid");
  requireWithin(root, sidecar);
  requireWithin(root, bundle);
  requireWithin(root, output);
  await requireNoLinkedAncestor(root, sidecar);
  await requireNoLinkedAncestor(root, bundle);
  await requireRegularFile(sidecar, "sidecar_invalid");
  await requireNonempty(sidecar);
  await requireDirectory(bundle, "bundle_invalid");

  const installerExtensions = target.includes("windows")
    ? new Set([".msi", ".exe"])
    : new Set([".dmg", ".pkg"]);
  const installers = (await listFiles(bundle))
    .filter((path) => installerExtensions.has(extname(path).toLowerCase()))
    .sort();
  if (installers.length < 1) throw new Error("installer_missing");

  const artifacts = [sidecar, ...installers];
  const components = [];
  const checksumLines = [];
  for (const path of artifacts) {
    await requireNoLinkedAncestor(root, path);
    await requireRegularFile(path, "artifact_invalid");
    await requireNonempty(path);
    const digest = sha256(await readFile(path));
    const name = relative(root, path).split(sep).join("/");
    checksumLines.push(`${digest}  ${name}`);
    components.push({
      type: "application",
      name,
      hashes: [{ alg: "SHA-256", content: digest }],
    });
  }

  await mkdir(output, { recursive: true });
  const checksumPath = join(output, `smartcat-${target}.sha256`);
  const sbomPath = join(output, `smartcat-${target}.artifacts.cdx.json`);
  await atomicWrite(checksumPath, `${checksumLines.join("\n")}\n`);
  await atomicWrite(
    sbomPath,
    `${JSON.stringify(
      {
        bomFormat: "CycloneDX",
        specVersion: "1.6",
        version: 1,
        metadata: {
          component: {
            type: "application",
            name: "BYOK Translator release artifacts",
          },
        },
        components,
      },
      null,
      2,
    )}\n`,
  );
  validateCycloneDxDocument(JSON.parse(await readFile(sbomPath, "utf8")), {
    minimumComponents: 2,
  });
  return { checksumPath, sbomPath, artifacts };
}

export async function verifyArtifactChecksums({ repositoryRoot, checksumPath }) {
  const root = resolve(repositoryRoot);
  const checksum = resolve(checksumPath);
  requireWithin(root, checksum);
  await requireNoLinkedAncestor(root, checksum);
  await requireRegularFile(checksum, "checksum_invalid");
  const lines = (await readFile(checksum, "utf8")).trimEnd().split("\n");
  if (lines.length < 2) throw new Error("checksum_subjects_insufficient");
  const names = new Set();
  for (const line of lines) {
    const match = /^([0-9a-f]{64})  ([^\r\n]+)$/.exec(line);
    if (!match) throw new Error("checksum_format_invalid");
    const [, expected, name] = match;
    if (
      isAbsolute(name) ||
      name.includes("\\") ||
      name.split("/").some((part) => part === "" || part === "." || part === "..") ||
      names.has(name)
    ) {
      throw new Error("checksum_path_invalid");
    }
    names.add(name);
    const path = resolve(root, ...name.split("/"));
    requireWithin(root, path);
    await requireNoLinkedAncestor(root, path);
    await requireRegularFile(path, "artifact_invalid");
    await requireNonempty(path);
    if (sha256(await readFile(path)) !== expected) {
      throw new Error("artifact_checksum_mismatch");
    }
  }
  return names.size;
}

export function validateCycloneDxDocument(
  document,
  { minimumComponents = 2, requireDependencyGraph = false } = {},
) {
  if (
    !document ||
    typeof document !== "object" ||
    document.bomFormat !== "CycloneDX" ||
    typeof document.specVersion !== "string" ||
    !Number.isInteger(document.version) ||
    document.version < 1
  ) {
    throw new Error("sbom_format_invalid");
  }
  if (!Array.isArray(document.components)) throw new Error("sbom_components_missing");
  if (document.components.length < minimumComponents) {
    throw new Error("sbom_components_insufficient");
  }
  for (const component of document.components) {
    if (!component || typeof component !== "object" || typeof component.name !== "string" || component.name.length < 1) {
      throw new Error("sbom_component_invalid");
    }
  }
  if (requireDependencyGraph) {
    if (
      !Array.isArray(document.dependencies) ||
      document.dependencies.length < 1 ||
      !document.dependencies.some(
        (dependency) => Array.isArray(dependency?.dependsOn) && dependency.dependsOn.length > 0,
      )
    ) {
      throw new Error("sbom_dependency_graph_missing");
    }
  }
  return document;
}

async function listFiles(root) {
  const files = [];
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    const metadata = await lstat(path);
    if (metadata.isSymbolicLink()) throw new Error("artifact_link");
    if (metadata.isDirectory()) files.push(...(await listFiles(path)));
    else if (metadata.isFile()) files.push(path);
    else throw new Error("artifact_invalid");
  }
  return files;
}

async function requireNoLinkedAncestor(root, path) {
  const rel = relative(root, path);
  let current = root;
  for (const part of rel.split(sep)) {
    if (!part) continue;
    current = join(current, part);
    const metadata = await lstat(current);
    if (metadata.isSymbolicLink()) throw new Error("artifact_link");
  }
}

async function requireRegularFile(path, code) {
  const metadata = await lstat(path);
  if (!metadata.isFile() || metadata.isSymbolicLink()) throw new Error(code);
}

async function requireDirectory(path, code) {
  const metadata = await lstat(path);
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) throw new Error(code);
}

async function requireNonempty(path) {
  const metadata = await lstat(path);
  if (metadata.size < 1) throw new Error("artifact_empty");
}

function requireWithin(root, path) {
  const rel = relative(root, path);
  if (rel === ".." || rel.startsWith(`..${sep}`) || isAbsolute(rel)) {
    throw new Error("artifact_path_invalid");
  }
}

function requireTarget(target) {
  if (!TARGETS.has(target)) throw new Error("target_unsupported");
}

async function atomicCopy(source, destination) {
  await mkdir(dirname(destination), { recursive: true });
  const temporary = `${destination}.tmp-${process.pid}`;
  await copyFile(source, temporary);
  await rename(temporary, destination);
}

async function atomicWrite(path, contents) {
  await mkdir(dirname(path), { recursive: true });
  const temporary = `${path}.tmp-${process.pid}`;
  await writeFile(temporary, contents, { flag: "wx", mode: 0o600 });
  await rename(temporary, path);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
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
    child.on("error", () => reject(new Error("sbom_command_failed")));
    child.on("exit", (code) =>
      code === 0 ? resolvePromise() : reject(new Error("sbom_command_failed")),
    );
  });
}

async function main() {
  const mode = process.argv[2];
  if (mode === "create-artifacts") {
    await createArtifactEvidence({
      repositoryRoot: process.cwd(),
      target: argument("--target"),
      sidecarPath: argument("--sidecar"),
      bundleRoot: argument("--bundle-root"),
      outputRoot: argument("--output-root"),
    });
    return;
  }
  if (mode === "generate-cargo") {
    await generateCargoDependencySbom({
      manifestPath: argument("--manifest"),
      lockPath: argument("--lock"),
      target: argument("--target"),
      outputStem: argument("--output-stem"),
      destinationPath: argument("--destination"),
    });
    return;
  }
  if (mode === "verify") {
    const checksumIndex = process.argv.indexOf("--checksums");
    if (checksumIndex >= 0) {
      const checksumPath = process.argv[checksumIndex + 1];
      if (!checksumPath || checksumPath.startsWith("--")) throw new Error("argument_missing");
      await verifyArtifactChecksums({ repositoryRoot: process.cwd(), checksumPath });
    }
    for (const descriptor of argumentsFor("--dependency")) {
      validateCycloneDxDocument(JSON.parse(await readFile(descriptor, "utf8")), {
        minimumComponents: 2,
        requireDependencyGraph: true,
      });
    }
    for (const descriptor of argumentsFor("--inventory")) {
      validateCycloneDxDocument(JSON.parse(await readFile(descriptor, "utf8")), {
        minimumComponents: 2,
      });
    }
    return;
  }
  throw new Error("mode_invalid");
}

function argument(name) {
  const index = process.argv.indexOf(name);
  const value = index >= 0 ? process.argv[index + 1] : undefined;
  if (!value || value.startsWith("--")) throw new Error("argument_missing");
  return value;
}

function argumentsFor(name) {
  const values = [];
  for (let index = 0; index < process.argv.length; index += 1) {
    if (process.argv[index] === name) {
      const value = process.argv[index + 1];
      if (!value || value.startsWith("--")) throw new Error("argument_missing");
      values.push(value);
    }
  }
  if (values.length < 1) throw new Error("argument_missing");
  return values;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main().catch((error) => {
    process.stderr.write(`release-evidence: ${error?.message ?? "failed"}\n`);
    process.exitCode = 1;
  });
}
