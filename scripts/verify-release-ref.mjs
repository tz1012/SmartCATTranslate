import { execFileSync } from 'node:child_process';
import { appendFile } from 'node:fs/promises';

const tag = process.env.RELEASE_TAG?.trim();
const match = /^app-v((?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?)$/.exec(tag ?? '');
if (!match) fail('release tag must exactly match app-vSEMVER');

const ref = `refs/tags/${tag}`;
let sha;
try {
  sha = git(['rev-parse', '--verify', `${ref}^{commit}`]).trim();
} catch {
  fail('release tag does not exist locally; branches and SHA inputs are rejected');
}

const expected = match[1];
const packageVersion = JSON.parse(show('package.json')).version;
const tauriVersion = JSON.parse(show('src-tauri/tauri.conf.json')).version;
const cargoVersion = /^version\s*=\s*"([^"]+)"/m.exec(show('src-tauri/Cargo.toml'))?.[1];
for (const [source, version] of [['package.json', packageVersion], ['tauri.conf.json', tauriVersion], ['Cargo.toml', cargoVersion]]) {
  if (version !== expected) fail(`${source} version ${version ?? '<missing>'} does not match tag ${expected}`);
}

if (process.env.GITHUB_OUTPUT) {
  await appendFile(process.env.GITHUB_OUTPUT, `release_tag=${tag}\nrelease_sha=${sha}\n`);
}
process.stdout.write(`verified release tag ${tag} and synchronized version ${expected}\n`);

function show(path) { return git(['show', `${ref}:${path}`]); }
function git(args) { return execFileSync('git', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }); }
function fail(message) { process.stderr.write(`verify-release-ref: ${message}\n`); process.exit(1); }
