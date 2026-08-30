param([string]$OutputRoot = '')
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Windows packaging must run on Windows.' }
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $OutputRoot) { $OutputRoot = Join-Path ([IO.Path]::GetTempPath()) ('smartcat-unsigned-package-' + [guid]::NewGuid()) }
$output = [IO.Path]::GetFullPath($OutputRoot)
New-Item -ItemType Directory -Path $output -Force | Out-Null
$env:SMARTCAT_ACCEPTANCE_ROOT = Join-Path $output 'app-data'
$env:LOCALAPPDATA = Join-Path $env:SMARTCAT_ACCEPTANCE_ROOT 'LocalAppData'
$env:APPDATA = Join-Path $env:SMARTCAT_ACCEPTANCE_ROOT 'RoamingAppData'
New-Item -ItemType Directory -Path $env:LOCALAPPDATA,$env:APPDATA -Force | Out-Null
Push-Location $repo
try {
  pnpm release:assets:verify
  pnpm runtime:build -- --target x86_64-pc-windows-msvc
  pnpm tauri build --config src-tauri/tauri.runtime.conf.json --target x86_64-pc-windows-msvc --bundles msi,nsis
  $bundles = Join-Path $repo 'src-tauri\target\x86_64-pc-windows-msvc\release\bundle'
  Copy-Item -LiteralPath $bundles -Destination (Join-Path $output 'UNSIGNED-bundles') -Recurse
  Get-ChildItem -LiteralPath (Join-Path $output 'UNSIGNED-bundles') -Recurse -File | Get-FileHash -Algorithm SHA256 | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS.json')
} finally { Pop-Location }
Write-Output "Unsigned Windows prerelease staged at $output. No user document directory was read, moved, or removed."
