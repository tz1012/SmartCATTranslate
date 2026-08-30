import { spawn } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

for (const name of ['TAURI_UPDATER_PUBLIC_KEY', 'TAURI_SIGNING_PRIVATE_KEY']) {
  if (!process.env[name]?.trim()) fail(`${name} is required`);
}
const root = await mkdtemp(join(tmpdir(), 'smartcat-updater-key-'));
const canary = join(root, 'canary.txt');
try {
  await writeFile(canary, `SmartCAT updater signing preflight ${process.env.GITHUB_RUN_ID ?? 'local'}\n`, { mode: 0o600 });
  await run('pnpm', ['tauri', 'signer', 'sign', canary]);
  await run('cargo', ['run', '--locked', '--quiet', '--manifest-path', 'src-tauri/Cargo.toml', '--bin', 'verify-updater-key', '--', canary, `${canary}.sig`]);
  process.stdout.write('updater public/private key pairing verified without persisting key material\n');
} finally {
  await rm(root, { recursive: true, force: true });
}

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { shell: process.platform === 'win32' && command === 'pnpm', windowsHide: true, stdio: ['ignore', 'inherit', 'inherit'] });
    child.on('error', reject);
    child.on('exit', (code) => code === 0 ? resolve() : reject(new Error(`${command} exited ${code}`)));
  });
}
function fail(message) { process.stderr.write(`verify-updater-keypair: ${message}\n`); process.exit(1); }
