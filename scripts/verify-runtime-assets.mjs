import { createHash } from 'node:crypto';
import { lstat, readFile, readdir } from 'node:fs/promises';
import { basename, join, resolve } from 'node:path';

const root = resolve(new URL('..', import.meta.url).pathname.replace(/^\/(?:[A-Za-z]:)/, (m) => m.slice(1)));
const manifest = JSON.parse(await readFile(join(root, 'src-tauri/resources/codex-runtime.json'), 'utf8'));
if (!/^\d+\.\d+\.\d+$/.test(manifest.version) || !Array.isArray(manifest.runtimes) || manifest.runtimes.length !== 3) fail('codex_manifest_invalid');
const expectedTargets = new Set(['x86_64-pc-windows-msvc', 'x86_64-apple-darwin', 'aarch64-apple-darwin']);
for (const runtime of manifest.runtimes) {
  if (!expectedTargets.delete(runtime.target) || !runtime.url.startsWith(`https://github.com/openai/codex/releases/download/${manifest.tag}/`) || !/^[0-9a-f]{64}$/.test(runtime.sha256) || !Number.isSafeInteger(runtime.size) || runtime.size < 1) fail('codex_runtime_pin_invalid');
}
if (expectedTargets.size) fail('codex_runtime_target_missing');

const pins = new Map([
  ['NotoSans-Variable.ttf', 'bfb7bb691513f12e734dc346c03a03f784912432d7e3fa8e56efcf906fe86b3d'],
  ['LICENSE.txt', 'e2e177a32561584d4fc13aaa3cd8e53758a12910f013fe9ca125419111722029'],
]);
for (const [name, expected] of pins) {
  const path = join(root, 'tests/fixtures/fonts', name);
  const metadata = await lstat(path);
  if (!metadata.isFile() || metadata.isSymbolicLink() || sha(await readFile(path)) !== expected) fail(`font_asset_invalid:${name}`);
}

const resources = await walk(join(root, 'src-tauri/resources'));
if (resources.some((path) => /pdfium/i.test(basename(path)))) fail('pdfium_must_not_be_bundled');
const builtManifestPath = join(root, 'src-tauri/resources/smartcat-codex-runtime.json');
try {
  const built = JSON.parse(await readFile(builtManifestPath, 'utf8'));
  if (typeof built.binary !== 'string' || basename(built.binary) !== built.binary) fail('built_runtime_binary_invalid');
  const binary = join(root, 'src-tauri/binaries', built.binary);
  if (!/^[0-9a-f]{64}$/.test(built.sha256) || sha(await readFile(binary)) !== built.sha256) fail('built_runtime_checksum_invalid');
} catch (error) {
  if (error?.code !== 'ENOENT') throw error;
  if (process.env.REQUIRE_BUILT_RUNTIME === '1') fail('built_runtime_manifest_missing');
}
process.stdout.write('runtime assets verified; PDFium is not bundled\n');

async function walk(dir) { const out=[]; for (const entry of await readdir(dir,{withFileTypes:true})) { const path=join(dir,entry.name); if(entry.isSymbolicLink()) fail('resource_link'); if(entry.isDirectory()) out.push(...await walk(path)); else out.push(path); } return out; }
function sha(bytes) { return createHash('sha256').update(bytes).digest('hex'); }
function fail(code) { throw new Error(code); }
