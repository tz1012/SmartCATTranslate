import { spawn } from 'node:child_process';
import { cp, mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { validateSpdxExpression } from './license-policy.mjs';

const output = resolve(process.argv.slice(2).find((value) => value !== '--') ?? 'artifacts/licenses');
const exceptions = JSON.parse(await readFile('scripts/release-license-exceptions.json', 'utf8'));
const inventory = [];
const missing = [];
await mkdir(join(output, 'npm'), { recursive: true });
await mkdir(join(output, 'cargo'), { recursive: true });

const npmGroups = JSON.parse(await run('pnpm', ['licenses', 'list', '--json', '--prod']));
for (const entries of Object.values(npmGroups)) for (const entry of entries) {
  for (let index = 0; index < entry.versions.length; index += 1) {
    const version = entry.versions[index];
    await collect('npm', entry.name, version, entry.license, entry.paths[index] ?? entry.paths[0]);
  }
}
const cargo = JSON.parse(await run('cargo', ['metadata', '--locked', '--format-version', '1', '--filter-platform', rustTarget(), '--manifest-path', 'src-tauri/Cargo.toml']));
for (const pkg of cargo.packages.filter((value) => !cargo.workspace_members.includes(value.id))) {
  await collect('cargo', pkg.name, pkg.version, pkg.license, dirname(pkg.manifest_path), pkg.license_file);
}

await cp('tests/fixtures/fonts/LICENSE.txt', join(output, 'NotoSans-LICENSE.txt'));
const runtime = JSON.parse(await readFile('src-tauri/resources/codex-runtime.json', 'utf8'));
for (const name of ['LICENSE', 'NOTICE']) await cp(join('src-tauri/resources', name), join(output, `CODEX-RUNTIME-${name}.txt`));
inventory.push({ ecosystem: 'asset', name: 'Noto Sans', version: 'pinned-checksum', license: validateSpdxExpression('OFL-1.1', 'asset:Noto Sans'), files: ['NotoSans-LICENSE.txt'] });
inventory.push({ ecosystem: 'asset', name: 'Codex runtime', version: runtime.version, license: validateSpdxExpression(runtime.license?.spdx, 'asset:Codex runtime'), files: ['CODEX-RUNTIME-LICENSE.txt', 'CODEX-RUNTIME-NOTICE.txt'] });
if (missing.length) throw new Error(`license_evidence_missing:\n${missing.join('\n')}`);
inventory.sort((a, b) => `${a.ecosystem}:${a.name}@${a.version}`.localeCompare(`${b.ecosystem}:${b.name}@${b.version}`));
await writeFile(join(output, 'license-inventory.json'), `${JSON.stringify(inventory, null, 2)}\n`);
await writeFile(join(output, 'README.txt'), 'This directory contains the actual discovered license/notice texts for production npm and Cargo dependencies plus pinned Noto Sans and Codex runtime assets. Entries without shipped text require a reviewed exception in scripts/release-license-exceptions.json. PDFium is not bundled.\n');
process.stdout.write(`bundled license evidence for ${inventory.length} components\n`);

async function collect(ecosystem, name, version, expression, packageRoot, explicitFile) {
  const key = `${ecosystem}:${name}@${version}`;
  const exception = exceptions[key];
  if ((!expression || !String(expression).trim()) && !validException(exception)) { missing.push(`metadata:${key}`); return; }
  const license = validateSpdxExpression(exception?.license || expression, key);
  const candidates = [];
  if (explicitFile) candidates.push(resolve(packageRoot, explicitFile));
  for (const entry of await readdir(packageRoot, { withFileTypes: true })) {
    if (entry.isFile() && /^(?:licen[cs]e|copying|copyright|notice)(?:[._-].*)?$/i.test(entry.name)) candidates.push(join(packageRoot, entry.name));
  }
  const unique = [...new Set(candidates)];
  if (!unique.length && !validException(exception)) { missing.push(`text:${key}:${expression}`); return; }
  const directory = join(output, ecosystem, safe(`${name}@${version}`));
  await mkdir(directory, { recursive: true });
  const files = [];
  for (const source of unique) {
    const target = join(directory, safe(basename(source)));
    await cp(source, target); files.push(target.slice(output.length + 1).replaceAll('\\', '/'));
  }
  inventory.push({ ecosystem, name, version, license, files, exception: exception ?? undefined });
}
function validException(value) { return Boolean(value && typeof value.reason === 'string' && value.reason.trim() && typeof value.license === 'string' && value.license.trim()); }
function safe(value) { return value.replace(/[^A-Za-z0-9._@-]+/g, '_'); }
function rustTarget() { return process.env.RELEASE_TARGET || (process.platform === 'win32' ? 'x86_64-pc-windows-msvc' : process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin'); }
function run(command, args) { const useShell = process.platform === 'win32' && command === 'pnpm'; return new Promise((ok, bad) => { const child = spawn(command, args, { cwd: process.cwd(), shell: useShell, windowsHide: true, stdio: ['ignore', 'pipe', 'inherit'] }); let text = ''; child.stdout.on('data', (chunk) => text += chunk); child.on('error', bad); child.on('exit', (code) => code === 0 ? ok(text) : bad(new Error(`${command}_failed`))); }); }
