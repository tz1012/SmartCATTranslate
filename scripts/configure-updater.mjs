import { readFile, writeFile } from 'node:fs/promises';

const configPath = new URL('../src-tauri/tauri.conf.json', import.meta.url);
const repository = process.env.GITHUB_REPOSITORY?.trim();
const publicKey = process.env.TAURI_UPDATER_PUBLIC_KEY?.trim();

if (!repository || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) fail('GITHUB_REPOSITORY is required and must be owner/repository');
if (!publicKey || publicKey.length < 32 || /placeholder|example|private key/i.test(publicKey)) fail('TAURI_UPDATER_PUBLIC_KEY must be the real Minisign public key from the protected GitHub Environment');
if (/SECRET KEY|PRIVATE KEY/.test(publicKey)) fail('a private signing key must never be written to updater configuration');

const config = JSON.parse(await readFile(configPath, 'utf8'));
config.bundle.createUpdaterArtifacts = true;
config.plugins = {
  ...(config.plugins ?? {}),
  updater: {
    pubkey: publicKey,
    endpoints: [`https://github.com/${repository}/releases/latest/download/latest.json`],
  },
};
if (process.platform === 'win32') {
  const thumbprint = process.env.WINDOWS_CERTIFICATE_THUMBPRINT?.replace(/\s/g, '').toUpperCase();
  if (!thumbprint || !/^[0-9A-F]{40,64}$/.test(thumbprint)) fail('WINDOWS_CERTIFICATE_THUMBPRINT is required for a signed Windows release');
  config.bundle.windows = {
    ...(config.bundle.windows ?? {}),
    certificateThumbprint: thumbprint,
    digestAlgorithm: 'sha256',
    timestampUrl: 'http://timestamp.digicert.com',
  };
}
await writeFile(configPath, `${JSON.stringify(config, null, 2)}\n`, { encoding: 'utf8', mode: 0o600 });
process.stdout.write(`configured updater endpoint for ${repository}\n`);

function fail(message) { process.stderr.write(`configure-updater: ${message}\n`); process.exit(1); }
