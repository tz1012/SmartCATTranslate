import { readFile } from 'node:fs/promises';
import { parseDocument } from 'yaml';

const ci = await readFile('.github/workflows/ci.yml', 'utf8');
const release = await readFile('.github/workflows/release.yml', 'utf8');
const config = JSON.parse(await readFile('src-tauri/tauri.conf.json', 'utf8'));
const updaterConfigurator = await readFile('scripts/configure-updater.mjs', 'utf8');
for (const [name, source] of [['ci', ci], ['release', release]]) {
  const document = parseDocument(source, { prettyErrors: true });
  if (document.errors.length) fail(`${name}_yaml_invalid:${document.errors[0].message}`);
  if (!document.toJS()?.jobs) fail(`${name}_jobs_missing`);
}

for (const job of ['frontend:', 'rust:', 'records:', 'privacy:']) requireText(ci, job, `ci_job_missing:${job}`);
for (const gate of ['needs: [frontend, rust, records, privacy]', 'UNSIGNED-smartcat-', 'pnpm privacy:check', 'pnpm records:test']) requireText(ci, gate, `ci_policy_missing:${gate}`);
for (const row of [
  'windows-latest, target: x86_64-pc-windows-msvc, bundles: "msi,nsis"',
  'macos-13, target: x86_64-apple-darwin, bundles: "app,dmg"',
  'macos-14, target: aarch64-apple-darwin, bundles: "app,dmg"',
]) requireText(release, row, `release_matrix_missing:${row}`);
for (const policy of [
  'environment: release-signing',
  'node scripts/verify-release-secrets.mjs',
  'node scripts/configure-updater.mjs',
  'Import Windows code-signing certificate',
  'temporary keychain',
  'APPLE_ID:',
  'TAURI_SIGNING_PRIVATE_KEY:',
  'pnpm release:sbom',
  'pnpm release:licenses',
  'SHA256SUMS',
  'actions/attest-build-provenance@v3',
  'draft_release:',
  'needs: package',
  'gh release create "$RELEASE_TAG" --draft',
]) requireText(release, policy, `release_policy_missing:${policy}`);

const actionIndex = release.indexOf('uses: tauri-apps/tauri-action@v1');
if (actionIndex < 0) fail('tauri_action_missing');
for (const gate of ['pnpm test', 'pnpm build', 'cargo check --locked', 'pnpm records:test', 'pnpm privacy:check', 'pnpm release:assets:verify']) {
  const index = release.lastIndexOf(gate, actionIndex);
  if (index < 0 || index > actionIndex) fail(`release_gate_not_before_tauri:${gate}`);
}
if (config.plugins?.updater || config.bundle?.createUpdaterArtifacts) fail('local_updater_must_remain_disabled');
requireText(updaterConfigurator, 'https://github.com/${repository}/releases/latest/download/latest.json', 'github_updater_endpoint_missing');
requireText(updaterConfigurator, 'TAURI_UPDATER_PUBLIC_KEY', 'updater_public_key_environment_missing');
if (/placeholder-public-key|example\.com\/update|BEGIN (?:RSA |EC )?PRIVATE KEY/.test(`${ci}\n${release}\n${updaterConfigurator}`)) fail('fake_or_private_release_material');
process.stdout.write('workflow policy verified\n');

function requireText(text, value, code) { if (!text.includes(value)) fail(code); }
function fail(code) { process.stderr.write(`workflow-policy: ${code}\n`); process.exit(1); }
