import { createHash } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gunzipSync } from "node:zlib";

const VERSION = "0.144.4";
const TAG = "rust-v0.144.4";
const API_URL = `https://api.github.com/repos/openai/codex/releases/tags/${TAG}`;
const RELEASE_URL = `https://github.com/openai/codex/releases/tag/${TAG}`;
const RELEASE_DOWNLOAD_PREFIX = `https://github.com/openai/codex/releases/download/${TAG}/`;
const RELEASE_CDN_HOST = "release-assets.githubusercontent.com";
const LICENSE_URL = `https://raw.githubusercontent.com/openai/codex/${TAG}/LICENSE`;
const NOTICE_URL = `https://raw.githubusercontent.com/openai/codex/${TAG}/NOTICE`;
const USER_AGENT = "SmartCAT-Translate-Codex-Runtime-Pinner";
const MAX_ARCHIVE_BYTES = 1024 * 1024 * 1024;

const TARGETS = Object.freeze([
  Object.freeze({
    target: "x86_64-pc-windows-msvc",
    assetName: "codex-x86_64-pc-windows-msvc.exe.zip",
    archiveEntry: "codex-x86_64-pc-windows-msvc.exe",
    archiveEntries: Object.freeze([
      "codex-command-runner.exe",
      "codex-windows-sandbox-setup.exe",
      "codex-x86_64-pc-windows-msvc.exe",
    ]),
  }),
  Object.freeze({
    target: "aarch64-apple-darwin",
    assetName: "codex-aarch64-apple-darwin.tar.gz",
    archiveEntry: "codex-aarch64-apple-darwin",
    archiveEntries: Object.freeze(["codex-aarch64-apple-darwin"]),
  }),
  Object.freeze({
    target: "x86_64-apple-darwin",
    assetName: "codex-x86_64-apple-darwin.tar.gz",
    archiveEntry: "codex-x86_64-apple-darwin",
    archiveEntries: Object.freeze(["codex-x86_64-apple-darwin"]),
  }),
]);

export async function buildManifest(release, download) {
  requireReleaseIdentity(release);
  requireUniqueTargets();

  const runtimes = [];
  for (const target of TARGETS) {
    const matches = release.assets.filter((asset) => asset.name === target.assetName);
    if (matches.length !== 1) {
      throw new Error(matches.length === 0 ? "asset_missing" : "asset_ambiguous");
    }

    const asset = matches[0];
    const expectedUrl = `${RELEASE_DOWNLOAD_PREFIX}${target.assetName}`;
    if (asset.browser_download_url !== expectedUrl) {
      throw new Error("asset_url_rejected");
    }
    if (!Number.isSafeInteger(asset.size) || asset.size < 1) {
      throw new Error("asset_size_invalid");
    }

    const digestMatch = /^sha256:([0-9a-f]{64})$/.exec(asset.digest ?? "");
    if (!digestMatch) {
      throw new Error("asset_digest_invalid");
    }

    const bytes = Buffer.from(await download(asset));
    if (bytes.length !== asset.size) {
      throw new Error("asset_size_mismatch");
    }
    const actual = sha256(bytes);
    if (actual !== digestMatch[1]) {
      throw new Error("checksum_mismatch");
    }
    inspectArchive(bytes, target.assetName, target.archiveEntry);

    runtimes.push({
      target: target.target,
      url: expectedUrl,
      sha256: actual,
      archiveEntry: target.archiveEntry,
    });
  }

  if (runtimes.length !== 3 || new Set(runtimes.map((item) => item.target)).size !== 3) {
    throw new Error("duplicate_or_missing_target");
  }

  return {
    version: VERSION,
    tag: TAG,
    releaseUrl: RELEASE_URL,
    license: {
      spdx: "Apache-2.0",
      url: LICENSE_URL,
      noticeUrl: NOTICE_URL,
    },
    runtimes,
  };
}

export async function downloadOfficialAsset(initialUrl, requestOnce = requestOnceWithFetch) {
  requireOfficialAssetUrl(initialUrl);
  let currentUrl = initialUrl;

  for (let redirects = 0; redirects <= 1; redirects += 1) {
    const response = await requestOnce(currentUrl);
    if (isRedirect(response.status)) {
      if (redirects === 1 || !response.location) {
        throw new Error("redirect_count_rejected");
      }
      const nextUrl = new URL(response.location, currentUrl);
      if (nextUrl.protocol !== "https:" || nextUrl.hostname !== RELEASE_CDN_HOST) {
        throw new Error("redirect_host_rejected");
      }
      currentUrl = nextUrl.href;
      continue;
    }

    if (response.status < 200 || response.status >= 300) {
      throw new Error("asset_download_failed");
    }
    const finalUrl = new URL(currentUrl);
    const isInitialReleaseUrl = currentUrl === initialUrl;
    const isReleaseCdn =
      finalUrl.protocol === "https:" && finalUrl.hostname === RELEASE_CDN_HOST;
    if (!isInitialReleaseUrl && !isReleaseCdn) {
      throw new Error("final_url_rejected");
    }
    if (!response.bytes || response.bytes.length === 0) {
      throw new Error("asset_download_empty");
    }
    return Buffer.from(response.bytes);
  }

  throw new Error("redirect_count_rejected");
}

export function inspectArchive(bytes, assetName, expectedEntry) {
  const entries = assetName.endsWith(".zip")
    ? inspectZip(bytes)
    : assetName.endsWith(".tar.gz")
      ? inspectTarGz(bytes)
      : unsupportedArchive();
  const target = TARGETS.find((candidate) => candidate.assetName === assetName);
  if (!target || target.archiveEntry !== expectedEntry) {
    throw new Error("archive_entry_mismatch");
  }
  if (!entries.includes(expectedEntry)) {
    throw new Error("archive_entry_mismatch");
  }
  if (
    entries.length !== target.archiveEntries.length ||
    !target.archiveEntries.every((entry) => entries.includes(entry))
  ) {
    throw new Error("unsupported_archive_structure");
  }
}

function requireReleaseIdentity(release) {
  if (
    !release ||
    release.tag_name !== TAG ||
    release.html_url !== RELEASE_URL ||
    !Array.isArray(release.assets)
  ) {
    throw new Error("release_identity_mismatch");
  }
}

function requireUniqueTargets() {
  if (
    TARGETS.length !== 3 ||
    new Set(TARGETS.map((target) => target.target)).size !== 3 ||
    new Set(TARGETS.map((target) => target.assetName)).size !== 3
  ) {
    throw new Error("duplicate_or_missing_target");
  }
}

function requireOfficialAssetUrl(url) {
  const parsed = new URL(url);
  const assetName = decodeURIComponent(parsed.pathname.split("/").at(-1) ?? "");
  if (
    parsed.protocol !== "https:" ||
    parsed.origin !== "https://github.com" ||
    !url.startsWith(RELEASE_DOWNLOAD_PREFIX) ||
    parsed.search ||
    parsed.hash ||
    !TARGETS.some((target) => target.assetName === assetName) ||
    url !== `${RELEASE_DOWNLOAD_PREFIX}${assetName}`
  ) {
    throw new Error("asset_url_rejected");
  }
}

function inspectZip(bytes) {
  if (!Buffer.isBuffer(bytes) || bytes.length < 22) {
    throw new Error("unsupported_archive_structure");
  }
  const searchStart = Math.max(0, bytes.length - 65_557);
  let endOffset = -1;
  for (let offset = bytes.length - 22; offset >= searchStart; offset -= 1) {
    if (bytes.readUInt32LE(offset) === 0x06054b50) {
      endOffset = offset;
      break;
    }
  }
  if (endOffset < 0) {
    throw new Error("unsupported_archive_structure");
  }

  const disk = bytes.readUInt16LE(endOffset + 4);
  const centralDisk = bytes.readUInt16LE(endOffset + 6);
  const diskEntries = bytes.readUInt16LE(endOffset + 8);
  const totalEntries = bytes.readUInt16LE(endOffset + 10);
  const centralSize = bytes.readUInt32LE(endOffset + 12);
  const centralOffset = bytes.readUInt32LE(endOffset + 16);
  const commentLength = bytes.readUInt16LE(endOffset + 20);
  if (
    disk !== 0 ||
    centralDisk !== 0 ||
    diskEntries !== totalEntries ||
    totalEntries === 0xffff ||
    centralSize === 0xffffffff ||
    centralOffset === 0xffffffff ||
    endOffset + 22 + commentLength !== bytes.length ||
    centralOffset + centralSize !== endOffset
  ) {
    throw new Error("unsupported_archive_structure");
  }

  const entries = [];
  let offset = centralOffset;
  for (let index = 0; index < totalEntries; index += 1) {
    if (offset + 46 > endOffset || bytes.readUInt32LE(offset) !== 0x02014b50) {
      throw new Error("unsupported_archive_structure");
    }
    const flags = bytes.readUInt16LE(offset + 8);
    const method = bytes.readUInt16LE(offset + 10);
    const nameLength = bytes.readUInt16LE(offset + 28);
    const extraLength = bytes.readUInt16LE(offset + 30);
    const entryCommentLength = bytes.readUInt16LE(offset + 32);
    const localOffset = bytes.readUInt32LE(offset + 42);
    const nameStart = offset + 46;
    const nextOffset = nameStart + nameLength + extraLength + entryCommentLength;
    if (
      flags & 1 ||
      (method !== 0 && method !== 8) ||
      nextOffset > endOffset ||
      localOffset + 30 > centralOffset ||
      bytes.readUInt32LE(localOffset) !== 0x04034b50
    ) {
      throw new Error("unsupported_archive_structure");
    }
    const name = bytes.toString("utf8", nameStart, nameStart + nameLength);
    const localNameLength = bytes.readUInt16LE(localOffset + 26);
    const localName = bytes.toString(
      "utf8",
      localOffset + 30,
      localOffset + 30 + localNameLength,
    );
    requireSafeEntryName(name);
    if (name !== localName || name.endsWith("/")) {
      throw new Error("unsupported_archive_structure");
    }
    entries.push(name);
    offset = nextOffset;
  }
  if (offset !== endOffset) {
    throw new Error("unsupported_archive_structure");
  }
  return entries;
}

function inspectTarGz(bytes) {
  let tar;
  try {
    tar = gunzipSync(bytes, { maxOutputLength: MAX_ARCHIVE_BYTES });
  } catch {
    throw new Error("unsupported_archive_structure");
  }
  if (tar.length < 1536 || tar.length % 512 !== 0) {
    throw new Error("unsupported_archive_structure");
  }

  const entries = [];
  let offset = 0;
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) {
      if (!tar.subarray(offset).every((byte) => byte === 0)) {
        throw new Error("unsupported_archive_structure");
      }
      break;
    }

    const storedChecksum = parseTarOctal(header.subarray(148, 156));
    const checksumHeader = Buffer.from(header);
    checksumHeader.fill(0x20, 148, 156);
    const actualChecksum = checksumHeader.reduce((sum, byte) => sum + byte, 0);
    const size = parseTarOctal(header.subarray(124, 136));
    const type = header[156];
    if (storedChecksum !== actualChecksum || (type !== 0 && type !== 0x30)) {
      throw new Error("unsupported_archive_structure");
    }

    const name = readTarString(header.subarray(0, 100));
    const prefix = readTarString(header.subarray(345, 500));
    const fullName = prefix ? `${prefix}/${name}` : name;
    requireSafeEntryName(fullName);
    entries.push(fullName);

    const dataBlocks = Math.ceil(size / 512);
    offset += 512 + dataBlocks * 512;
    if (offset > tar.length) {
      throw new Error("unsupported_archive_structure");
    }
  }
  return entries;
}

function parseTarOctal(field) {
  const value = field.toString("ascii").replace(/\0.*$/, "").trim();
  if (!/^[0-7]+$/.test(value)) {
    throw new Error("unsupported_archive_structure");
  }
  const parsed = Number.parseInt(value, 8);
  if (!Number.isSafeInteger(parsed) || parsed < 0) {
    throw new Error("unsupported_archive_structure");
  }
  return parsed;
}

function readTarString(field) {
  return field.subarray(0, field.indexOf(0) < 0 ? field.length : field.indexOf(0)).toString("utf8");
}

function requireSafeEntryName(name) {
  if (
    !name ||
    name.includes("\\") ||
    name.startsWith("/") ||
    name.split("/").some((part) => part === "" || part === "." || part === "..")
  ) {
    throw new Error("unsupported_archive_structure");
  }
}

function unsupportedArchive() {
  throw new Error("unsupported_archive_structure");
}

function isRedirect(status) {
  return status === 301 || status === 302 || status === 303 || status === 307 || status === 308;
}

async function requestOnceWithFetch(url) {
  const response = await fetch(url, {
    redirect: "manual",
    headers: {
      Accept: "application/octet-stream",
      "User-Agent": USER_AGENT,
    },
  });
  return {
    status: response.status,
    location: response.headers.get("location") ?? undefined,
    bytes:
      response.status >= 200 && response.status < 300
        ? Buffer.from(await response.arrayBuffer())
        : undefined,
  };
}

async function fetchRelease() {
  const response = await fetch(API_URL, {
    redirect: "error",
    headers: {
      Accept: "application/vnd.github+json",
      "User-Agent": USER_AGENT,
      "X-GitHub-Api-Version": "2022-11-28",
    },
  });
  if (!response.ok || response.url !== API_URL) {
    throw new Error("release_metadata_failed");
  }
  return response.json();
}

async function fetchPinnedText(url) {
  const response = await fetch(url, {
    redirect: "error",
    headers: { "User-Agent": USER_AGENT },
  });
  if (!response.ok || response.url !== url) {
    throw new Error("license_provenance_failed");
  }
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length === 0) {
    throw new Error("license_provenance_failed");
  }
  return bytes;
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

async function main() {
  const release = await fetchRelease();
  const manifest = await buildManifest(release, (asset) =>
    downloadOfficialAsset(asset.browser_download_url),
  );
  const [license, notice] = await Promise.all([
    fetchPinnedText(LICENSE_URL),
    fetchPinnedText(NOTICE_URL),
  ]);

  const resources = resolve("src-tauri", "resources");
  await mkdir(resources, { recursive: true });
  await Promise.all([
    writeFile(resolve(resources, "codex-runtime.json"), `${JSON.stringify(manifest, null, 2)}\n`),
    writeFile(resolve(resources, "LICENSE"), license),
    writeFile(resolve(resources, "NOTICE"), notice),
  ]);
  console.log("Pinned Codex rust-v0.144.4 for 3 targets; checksums and archive entries verified.");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`pin-codex-runtime: ${error instanceof Error ? error.message : "unknown_error"}`);
    process.exitCode = 1;
  });
}
