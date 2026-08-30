const common = ['TAURI_UPDATER_PUBLIC_KEY', 'TAURI_SIGNING_PRIVATE_KEY', 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD'];
const platform = process.platform === 'win32'
  ? ['WINDOWS_CERTIFICATE', 'WINDOWS_CERTIFICATE_PASSWORD', 'WINDOWS_CERTIFICATE_THUMBPRINT']
  : ['APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD', 'APPLE_SIGNING_IDENTITY', 'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID'];
const missing = [...common, ...platform].filter((name) => !process.env[name]?.trim());
if (missing.length) {
  process.stderr.write(`release environment is missing required variables: ${missing.join(', ')}\n`);
  process.exit(1);
}
process.stdout.write('required release environment variables are present (values not printed)\n');
