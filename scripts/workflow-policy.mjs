import { readFile } from 'node:fs/promises';
import { parseDocument } from 'yaml';

const ci = await readFile('.github/workflows/ci.yml', 'utf8');
const release = await readFile('.github/workflows/release.yml', 'utf8');
const config = JSON.parse(await readFile('src-tauri/tauri.conf.json', 'utf8'));
const updaterConfigurator = await readFile('scripts/configure-updater.mjs', 'utf8');
const releaseRefVerifier = await readFile('scripts/verify-release-ref.mjs', 'utf8');
const windowsAcceptance = await readFile('tests/release/acceptance.ps1', 'utf8');
const macAcceptance = await readFile('tests/release/acceptance.sh', 'utf8');
for (const [name, source] of [['ci', ci], ['release', release]]) {
  const document = parseDocument(source, { prettyErrors: true });
  if (document.errors.length) fail(`${name}_yaml_invalid:${document.errors[0].message}`);
  if (!document.toJS()?.jobs) fail(`${name}_jobs_missing`);
  const retiredIntelRunner = ['macos', '13'].join('-');
  if (source.includes(retiredIntelRunner)) fail(`${name}_uses_retired_intel_runner`);
}

for (const job of ['frontend:', 'rust:', 'records:', 'privacy:']) requireText(ci, job, `ci_job_missing:${job}`);
for (const gate of ['needs: [frontend, rust, records, privacy]', 'UNSIGNED-smartcat-', 'pnpm privacy:check', 'pnpm records:test']) requireText(ci, gate, `ci_policy_missing:${gate}`);
for (const row of [
  'windows-latest, target: x86_64-pc-windows-msvc, bundles: "msi,nsis"',
  'macos-15-intel, target: x86_64-apple-darwin, bundles: "app,dmg"',
  'macos-14, target: aarch64-apple-darwin, bundles: "app,dmg"',
]) requireText(release, row, `release_matrix_missing:${row}`);
for (const policy of [
  'node scripts/verify-release-ref.mjs',
  'ref: "${{ needs.gates.outputs.release_sha }}"',
  'environment: release-signing',
  'node scripts/verify-release-secrets.mjs',
  'node scripts/configure-updater.mjs',
  'node scripts/verify-updater-keypair.mjs',
  'tests/release/acceptance.ps1 -MsiPath $msi.FullName -CiEphemeral',
  'tests/release/acceptance.sh "$dmg" --ci-ephemeral',
  'Assert Authenticode Valid on every Windows installer',
  'ditto -c -k --sequesterRsrc --keepParent',
  'pnpm release:sbom',
  'pnpm release:licenses',
  'actions/attest-build-provenance@v3',
  'needs: [gates, package]',
  'gh release create "$RELEASE_TAG" --draft',
]) requireText(release, policy, `release_policy_missing:${policy}`);

const firstTauri = release.indexOf('uses: tauri-apps/tauri-action@v1');
if (firstTauri < 0) fail('tauri_action_missing');
for (const gate of ['needs: gates', 'pnpm build', 'cargo check --locked', 'pnpm records:check', 'pnpm privacy:check', 'pnpm release:assets:verify', 'node scripts/verify-updater-keypair.mjs']) {
  const index = release.lastIndexOf(gate, firstTauri);
  if (index < 0 || index > firstTauri) fail(`release_gate_not_before_tauri:${gate}`);
}
const draftIndex = release.indexOf('\n  draft_release:');
for (const gate of ['Get-AuthenticodeSignature', "Status -ne 'Valid'", 'User Documents changed']) requireText(windowsAcceptance, gate, `windows_acceptance_assertion_missing:${gate}`);
for (const gate of ['codesign --verify --deep --strict', 'spctl --assess', 'xcrun stapler validate "$copied"', 'xcrun stapler validate "$dmg"']) requireText(macAcceptance, gate, `mac_acceptance_assertion_missing:${gate}`);
if (draftIndex < release.lastIndexOf('acceptance.sh')) fail('draft_release_must_follow_acceptance');
if (config.plugins?.updater || config.bundle?.createUpdaterArtifacts) fail('local_updater_must_remain_disabled');
requireText(updaterConfigurator, 'https://github.com/${repository}/releases/latest/download/latest.json', 'github_updater_endpoint_missing');
requireText(updaterConfigurator, 'TAURI_UPDATER_PUBLIC_KEY', 'updater_public_key_environment_missing');
for (const value of ['refs/tags/${tag}', 'package.json', 'src-tauri/Cargo.toml', 'src-tauri/tauri.conf.json']) requireText(releaseRefVerifier, value, `release_ref_verifier_missing:${value}`);
if (/placeholder-public-key|example\.com\/update|BEGIN (?:RSA |EC )?PRIVATE KEY/.test(`${ci}\n${release}\n${updaterConfigurator}`)) fail('fake_or_private_release_material');
process.stdout.write('workflow policy verified\n');

function requireText(text, value, code) { if (!text.includes(value)) fail(code); }
function fail(code) { process.stderr.write(`workflow-policy: ${code}\n`); process.exit(1); }
