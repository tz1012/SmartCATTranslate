import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { gzipSync } from "node:zlib";
import test from "node:test";

import {
  buildManifest,
  downloadOfficialAsset,
  inspectArchive,
  runCli,
} from "./pin-codex-runtime.mjs";

const TAG = "rust-v0.144.4";
const RELEASE_PREFIX = `https://github.com/openai/codex/releases/download/${TAG}/`;
const TARGETS = [
  {
    target: "x86_64-pc-windows-msvc",
    asset: "codex-x86_64-pc-windows-msvc.exe.zip",
    entry: "codex-x86_64-pc-windows-msvc.exe",
  },
  {
    target: "aarch64-apple-darwin",
    asset: "codex-aarch64-apple-darwin.tar.gz",
    entry: "codex-aarch64-apple-darwin",
  },
  {
    target: "x86_64-apple-darwin",
    asset: "codex-x86_64-apple-darwin.tar.gz",
    entry: "codex-x86_64-apple-darwin",
  },
];

test("buildManifest pins exactly the three required target archives", async () => {
  const archives = new Map([
    [TARGETS[0].asset, officialWindowsArchive()],
    [TARGETS[1].asset, tarGzWithSingleEntry(TARGETS[1].entry)],
    [TARGETS[2].asset, tarGzWithSingleEntry(TARGETS[2].entry)],
  ]);
  const release = releaseFixture(archives);

  const manifest = await buildManifest(release, async (asset) => archives.get(asset.name));

  assert.equal(manifest.version, "0.144.4");
  assert.equal(manifest.tag, TAG);
  assert.deepEqual(
    manifest.runtimes.map(({ target, url, archiveEntry }) => ({
      target,
      url,
      archiveEntry,
    })),
    TARGETS.map(({ target, asset, entry }) => ({
      target,
      url: `${RELEASE_PREFIX}${asset}`,
      archiveEntry: entry,
    })),
  );
});

test("buildManifest rejects an ambiguous duplicate target asset", async () => {
  const archives = new Map([
    [TARGETS[0].asset, officialWindowsArchive()],
    [TARGETS[1].asset, tarGzWithSingleEntry(TARGETS[1].entry)],
    [TARGETS[2].asset, tarGzWithSingleEntry(TARGETS[2].entry)],
  ]);
  const release = releaseFixture(archives);
  release.assets.push({ ...release.assets[0], id: 9999 });

  await assert.rejects(
    buildManifest(release, async (asset) => archives.get(asset.name)),
    /asset_ambiguous/,
  );
});

test("buildManifest rejects downloaded bytes that miss the API digest", async () => {
  const archives = new Map([
    [TARGETS[0].asset, officialWindowsArchive()],
    [TARGETS[1].asset, tarGzWithSingleEntry(TARGETS[1].entry)],
    [TARGETS[2].asset, tarGzWithSingleEntry(TARGETS[2].entry)],
  ]);
  const release = releaseFixture(archives);

  await assert.rejects(
    buildManifest(release, async (asset) => {
      const bytes = Buffer.from(archives.get(asset.name));
      if (asset.name === TARGETS[0].asset) {
        bytes[0] ^= 0xff;
      }
      return bytes;
    }),
    /checksum_mismatch/,
  );
});

test("inspectArchive rejects an archive whose only entry has the wrong name", () => {
  const archive = tarGzWithSingleEntry("different-entry");

  assert.throws(
    () => inspectArchive(archive, TARGETS[1].asset, TARGETS[1].entry),
    /archive_entry_mismatch/,
  );
});

test("inspectArchive accepts the exact official Windows helper layout", () => {
  const archive = zipWithEntries([
    "codex-command-runner.exe",
    "codex-windows-sandbox-setup.exe",
    TARGETS[0].entry,
  ]);

  assert.doesNotThrow(() => inspectArchive(archive, TARGETS[0].asset, TARGETS[0].entry));
});

test("inspectArchive rejects unsupported multi-entry archive structure", () => {
  const archive = zipWithEntries([TARGETS[0].entry, "unexpected-file"]);

  assert.throws(
    () => inspectArchive(archive, TARGETS[0].asset, TARGETS[0].entry),
    /unsupported_archive_structure/,
  );
});

test("downloadOfficialAsset permits only the GitHub release CDN final host", async () => {
  const initial = `${RELEASE_PREFIX}${TARGETS[0].asset}`;
  const cdn = "https://release-assets.githubusercontent.com/github-production-release-asset/1/file";
  const requests = [];
  const responses = [
    response(302, { location: cdn }),
    response(200, { body: Buffer.from("archive"), contentLength: 7 }),
  ];

  const bytes = await downloadOfficialAsset(initial, 7, async (url) => {
    requests.push(url);
    return responses.shift();
  });

  assert.deepEqual(requests, [initial, cdn]);
  assert.deepEqual(bytes, Buffer.from("archive"));
});

test("downloadOfficialAsset rejects any other redirect host", async () => {
  const initial = `${RELEASE_PREFIX}${TARGETS[0].asset}`;

  for (const location of [
    "https://downloads.example.invalid/codex.zip",
    "https://release-assets.githubusercontent.com:444/codex.zip",
    "https://attacker@release-assets.githubusercontent.com/codex.zip",
  ]) {
    await assert.rejects(
      downloadOfficialAsset(initial, 7, async () => response(302, { location })),
      /redirect_host_rejected/,
    );
  }
});

test("downloadOfficialAsset rejects a second redirect on the release CDN", async () => {
  const initial = `${RELEASE_PREFIX}${TARGETS[0].asset}`;
  const responses = [
    response(302, {
      location: "https://release-assets.githubusercontent.com/github-production-release-asset/1/first",
    }),
    response(302, {
      location: "https://release-assets.githubusercontent.com/github-production-release-asset/1/second",
    }),
    response(200, { body: Buffer.from("archive"), contentLength: 7 }),
  ];

  await assert.rejects(
    downloadOfficialAsset(initial, 7, async () => responses.shift()),
    /redirect_count_rejected/,
  );
});

test("downloadOfficialAsset rejects a zero-redirect success", async () => {
  const initial = `${RELEASE_PREFIX}${TARGETS[0].asset}`;

  await assert.rejects(
    downloadOfficialAsset(initial, 7, async () =>
      response(200, { body: Buffer.from("archive"), contentLength: 7 }),
    ),
    /redirect_count_rejected/,
  );
});

test("downloadOfficialAsset requires the exact final Content-Length", async () => {
  const initial = `${RELEASE_PREFIX}${TARGETS[0].asset}`;
  const cdn = "https://release-assets.githubusercontent.com/github-production-release-asset/1/file";
  for (const contentLength of [undefined, 6, 8]) {
    const responses = [
      response(302, { location: cdn }),
      response(200, { body: Buffer.from("archive"), contentLength }),
    ];
    await assert.rejects(
      downloadOfficialAsset(initial, 7, async () => responses.shift()),
      /content_length_rejected/,
    );
  }
});

test("downloadOfficialAsset aborts when streamed bytes exceed the expected size", async () => {
  const initial = `${RELEASE_PREFIX}${TARGETS[0].asset}`;
  const cdn = "https://release-assets.githubusercontent.com/github-production-release-asset/1/file";
  let chunksRead = 0;
  const body = {
    async *[Symbol.asyncIterator]() {
      chunksRead += 1;
      yield Buffer.from("archives");
      chunksRead += 1;
      yield Buffer.from("must-not-be-read");
    },
  };
  const responses = [response(302, { location: cdn }), response(200, { body, contentLength: 7 })];

  await assert.rejects(
    downloadOfficialAsset(initial, 7, async () => responses.shift()),
    /download_size_exceeded/,
  );
  assert.equal(chunksRead, 1);
});

test("buildManifest rejects oversized API asset metadata before download", async () => {
  const archives = new Map([
    [TARGETS[0].asset, officialWindowsArchive()],
    [TARGETS[1].asset, tarGzWithSingleEntry(TARGETS[1].entry)],
    [TARGETS[2].asset, tarGzWithSingleEntry(TARGETS[2].entry)],
  ]);
  const release = releaseFixture(archives);
  release.assets[0].size = 200 * 1024 * 1024;
  let downloaded = false;

  await assert.rejects(
    buildManifest(release, async () => {
      downloaded = true;
      return Buffer.alloc(0);
    }),
    /asset_size_invalid/,
  );
  assert.equal(downloaded, false);
});

test("inspectArchive rejects a local header that disagrees with the central directory", () => {
  const archive = officialWindowsArchive();
  archive.writeUInt16LE(8, 8);

  assert.throws(
    () => inspectArchive(archive, TARGETS[0].asset, TARGETS[0].entry),
    /unsupported_archive_structure/,
  );
});

test("inspectArchive rejects ZIP64 sentinels", () => {
  const archive = officialWindowsArchive();
  archive.writeUInt16LE(0xffff, archive.length - 12);

  assert.throws(
    () => inspectArchive(archive, TARGETS[0].asset, TARGETS[0].entry),
    /unsupported_archive_structure/,
  );
});

test("inspectArchive bounds gzip expansion before accepting entries", () => {
  const archive = tarGzWithSingleEntry(TARGETS[1].entry);

  assert.throws(
    () => inspectArchive(archive, TARGETS[1].asset, TARGETS[1].entry, 1024),
    /archive_expansion_exceeded/,
  );
});

test("runCli sanitizes filesystem failures without paths or arbitrary messages", async () => {
  const output = [];
  const error = Object.assign(new Error("access denied at C:\\Users\\private\\runtime.json"), {
    code: "EACCES",
  });

  const exitCode = await runCli(
    async () => {
      throw error;
    },
    (message) => output.push(message),
  );

  assert.equal(exitCode, 1);
  assert.deepEqual(output, ["pin-codex-runtime: filesystem_write_failed"]);
});

function releaseFixture(archives) {
  return {
    tag_name: TAG,
    html_url: `https://github.com/openai/codex/releases/tag/${TAG}`,
    assets: TARGETS.map(({ asset }) => ({
      id: asset.length,
      name: asset,
      browser_download_url: `${RELEASE_PREFIX}${asset}`,
      digest: `sha256:${sha256(archives.get(asset))}`,
      size: archives.get(asset).length,
    })),
  };
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function response(status, { location, body, contentLength } = {}) {
  const stream =
    body && typeof body[Symbol.asyncIterator] === "function"
      ? body
      : body
        ? {
            async *[Symbol.asyncIterator]() {
              yield body;
            },
          }
        : undefined;
  return { status, location, body: stream, contentLength };
}

function officialWindowsArchive() {
  return zipWithEntries([
    "codex-command-runner.exe",
    "codex-windows-sandbox-setup.exe",
    TARGETS[0].entry,
  ]);
}

function zipWithEntries(names) {
  const localParts = [];
  const centralParts = [];
  let localOffset = 0;
  for (const name of names) {
    const encoded = Buffer.from(name);
    const local = Buffer.alloc(30 + encoded.length);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0, 6);
    local.writeUInt16LE(0, 8);
    local.writeUInt16LE(encoded.length, 26);
    encoded.copy(local, 30);
    localParts.push(local);

    const central = Buffer.alloc(46 + encoded.length);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(0, 8);
    central.writeUInt16LE(0, 10);
    central.writeUInt16LE(encoded.length, 28);
    central.writeUInt32LE(localOffset, 42);
    encoded.copy(central, 46);
    centralParts.push(central);
    localOffset += local.length;
  }

  const central = Buffer.concat(centralParts);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(names.length, 8);
  end.writeUInt16LE(names.length, 10);
  end.writeUInt32LE(central.length, 12);
  end.writeUInt32LE(localOffset, 16);
  return Buffer.concat([...localParts, central, end]);
}

function tarGzWithSingleEntry(name) {
  const header = Buffer.alloc(512);
  header.write(name, 0, 100, "utf8");
  header.write("0000755\0", 100, 8, "ascii");
  header.write("0000000\0", 108, 8, "ascii");
  header.write("0000000\0", 116, 8, "ascii");
  header.write("00000000000\0", 124, 12, "ascii");
  header.write("00000000000\0", 136, 12, "ascii");
  header.fill(0x20, 148, 156);
  header[156] = "0".charCodeAt(0);
  header.write("ustar\0", 257, 6, "ascii");
  header.write("00", 263, 2, "ascii");
  const checksum = header.reduce((sum, byte) => sum + byte, 0);
  header.write(`${checksum.toString(8).padStart(6, "0")}\0 `, 148, 8, "ascii");
  return gzipSync(Buffer.concat([header, Buffer.alloc(1024)]), { mtime: 0 });
}
