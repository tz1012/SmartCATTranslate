import { readFile } from 'node:fs/promises';
import { parseDocument } from 'yaml';

const ci = await readFile('.github/workflows/ci.yml', 'utf8');
const release = await readFile('.github/workflows/release.yml', 'utf8');
const config = JSON.parse(await readFile('src-tauri/tauri.conf.json', 'utf8'));
const cargoManifest = await readFile('src-tauri/Cargo.toml', 'utf8');
const updaterConfigurator = await readFile('scripts/configure-updater.mjs', 'utf8');
const releaseRefVerifier = await readFile('scripts/verify-release-ref.mjs', 'utf8');
const updaterManifest = await readFile('scripts/generate-updater-manifest.mjs', 'utf8');
const licensePolicy = await readFile('scripts/license-policy.mjs', 'utf8');
const sbomGenerator = await readFile('scripts/generate-sbom.mjs', 'utf8');
const windowsAcceptance = await readFile('tests/release/acceptance.ps1', 'utf8');
const macAcceptance = await readFile('tests/release/acceptance.sh', 'utf8');
const appSource = await readFile('src/app/App.tsx', 'utf8');
const rustRuntime = `${await readFile('src-tauri/src/lib.rs', 'utf8')}\n${await readFile('src-tauri/src/commands/update.rs', 'utf8')}`;
requireText(cargoManifest, 'default-run = "smartcat-translate"', 'cargo_default_run_missing');
requireText(cargoManifest, 'name = "verify-updater-key"', 'updater_verifier_binary_missing');
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
  "Extension -In '.msi','.exe','.zip','.sig'",
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
const manifestIndex = release.indexOf('node scripts/generate-updater-manifest.mjs release-assets');
const checksumIndex = release.indexOf('> release-assets/SHA256SUMS');
const attestationIndex = release.indexOf('subject-checksums: release-assets/SHA256SUMS');
const uploadIndex = release.indexOf('find release-assets -type f -print0');
if (!(manifestIndex >= 0 && manifestIndex < checksumIndex && checksumIndex < attestationIndex && attestationIndex < uploadIndex)) fail('manifest_checksum_attestation_upload_order_invalid');
requireText(release, 'find release-assets -type f ! -name SHA256SUMS', 'checksum_self_reference_exclusion_missing');
for (const value of ["/\\.nsis\\.zip$/", "/\\.msi\\.zip$/", "files.includes(`${file}.sig`)", "join(root,'latest.json')"]) requireText(updaterManifest, value, `updater_manifest_policy_missing:${value}`);
if (updaterManifest.includes('.exe') || updaterManifest.includes('/\\.msi$/')) fail('raw_windows_installer_updater_feed_forbidden');
if (updaterManifest.indexOf('/\\.nsis\\.zip$/') > updaterManifest.indexOf('/\\.msi\\.zip$/')) fail('nsis_updater_zip_must_be_preferred');
for (const value of ["$env:CI -ne 'true'", "$env:GITHUB_ACTIONS -ne 'true'"]) requireText(windowsAcceptance, value, `windows_ephemeral_guard_missing:${value}`);
for (const value of ['"${CI:-}" != true', '"${GITHUB_ACTIONS:-}" != true']) requireText(macAcceptance, value, `mac_ephemeral_guard_missing:${value}`);
const winOwnedInit = windowsAcceptance.indexOf('$appDataOwned = $false');
const winPrecondition = windowsAcceptance.indexOf('if (Test-Path -LiteralPath $expectedAppData)');
const winOwnedSet = windowsAcceptance.indexOf('$appDataOwned = $true');
const winOwnedCleanup = windowsAcceptance.lastIndexOf('if ($appDataOwned -and');
if (!(winOwnedInit >= 0 && winOwnedInit < winPrecondition && winPrecondition < winOwnedSet && winOwnedSet < winOwnedCleanup)) fail('windows_app_data_ownership_order_invalid');
for (const value of ['$installedByThisRun = $false', '$installedByThisRun = $true', 'if ($installedByThisRun)', 'local-data-key.com.smartcat.translate', '[Environment+SpecialFolder]::LocalApplicationData']) requireText(windowsAcceptance, value, `windows_cleanup_ownership_missing:${value}`);
if (windowsAcceptance.includes("Join-Path $env:LOCALAPPDATA 'com.smartcat.translate'")) fail('windows_cleanup_must_not_trust_environment_app_data');
const macOwnedInit = macAcceptance.indexOf('app_data_owned=false; copied_by_this_run=false');
const macCleanupGuard = macAcceptance.indexOf('if [[ "$app_data_owned" == true');
const macPrecondition = macAcceptance.indexOf('[[ ! -e "$expected_app_data" ]]');
const macOwnedSet = macAcceptance.indexOf('app_data_owned=true');
if (!(macOwnedInit >= 0 && macOwnedInit < macCleanupGuard && macCleanupGuard < macPrecondition && macPrecondition < macOwnedSet)) fail('mac_app_data_ownership_order_invalid');
for (const value of ['copied_by_this_run=false', 'copied_by_this_run=true', '"$copied_by_this_run" == true', 'NFSHomeDirectory', 'security find-generic-password']) requireText(macAcceptance, value, `mac_cleanup_ownership_missing:${value}`);
if (/SMARTCAT_ACCEPTANCE|\[0xA5; 32\]/.test(`${rustRuntime}\n${windowsAcceptance}\n${macAcceptance}`)) fail('production_acceptance_backdoor_forbidden');
for (const command of ['get_lifecycle_status', 'get_account', 'get_settings', 'get_privacy_status', 'list_history', 'list_recoverable_jobs', 'await delay(1_500)', 'mark_app_healthy']) requireText(appSource, command, `healthy_gate_missing:${command}`);
for (const value of ['spdx-expression-parse', 'JsonValidator', 'Version.v1dot6']) requireText(licensePolicy, value, `license_validator_missing:${value}`);
requireText(sbomGenerator, 'validateCycloneDx16(jsonText)', 'cyclonedx_structural_validation_missing');
if (config.plugins?.updater || config.bundle?.createUpdaterArtifacts) fail('local_updater_must_remain_disabled');
requireText(updaterConfigurator, 'https://github.com/${repository}/releases/latest/download/latest.json', 'github_updater_endpoint_missing');
requireText(updaterConfigurator, 'TAURI_UPDATER_PUBLIC_KEY', 'updater_public_key_environment_missing');
for (const value of ['refs/tags/${tag}', 'package.json', 'src-tauri/Cargo.toml', 'src-tauri/tauri.conf.json']) requireText(releaseRefVerifier, value, `release_ref_verifier_missing:${value}`);
if (/placeholder-public-key|example\.com\/update|BEGIN (?:RSA |EC )?PRIVATE KEY/.test(`${ci}\n${release}\n${updaterConfigurator}`)) fail('fake_or_private_release_material');
process.stdout.write('workflow policy verified\n');

function requireText(text, value, code) { if (!text.includes(value)) fail(code); }
function fail(code) { process.stderr.write(`workflow-policy: ${code}\n`); process.exit(1); }
