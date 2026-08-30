const scopes = {
  common: ['TAURI_UPDATER_PUBLIC_KEY', 'TAURI_SIGNING_PRIVATE_KEY', 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD'],
  windows: ['WINDOWS_CERTIFICATE', 'WINDOWS_CERTIFICATE_PASSWORD', 'WINDOWS_CERTIFICATE_THUMBPRINT'],
  macos: ['APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD', 'APPLE_SIGNING_IDENTITY', 'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID'],
};
const scope = process.env.RELEASE_SECRET_SCOPE;
if (!Object.hasOwn(scopes, scope)) {
  process.stderr.write('RELEASE_SECRET_SCOPE must be common, windows, or macos\n');
  process.exit(1);
}
const missing = scopes[scope].filter((name) => !process.env[name]?.trim());
if (missing.length) {
  process.stderr.write(`release environment is missing required variables: ${missing.join(', ')}\n`);
  process.exit(1);
}
process.stdout.write(`${scope} release variables are present (values not printed)\n`);
