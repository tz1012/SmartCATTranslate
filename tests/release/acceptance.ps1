param([Parameter(Mandatory=$true)][string]$MsiPath, [switch]$CiEphemeral)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Windows acceptance must run on Windows.' }
$msi = (Resolve-Path -LiteralPath $MsiPath).Path
if ([IO.Path]::GetExtension($msi) -ne '.msi') { throw 'Acceptance requires an MSI artifact.' }
if ($CiEphemeral -and $env:CI -ne 'true') { throw '-CiEphemeral is allowed only when CI=true.' }
$root = Join-Path ([IO.Path]::GetTempPath()) ('smartcat-release-acceptance-' + [guid]::NewGuid())
$install = Join-Path $root 'app'; $data = Join-Path $root 'test-data'
New-Item -ItemType Directory -Path $install,$data -Force | Out-Null
$documents = [Environment]::GetFolderPath('MyDocuments')
function Get-DocumentsSnapshot {
  if (-not (Test-Path -LiteralPath $documents)) { return @() }
  return @(Get-ChildItem -LiteralPath $documents -File -Recurse -ErrorAction Stop | Get-FileHash -Algorithm SHA256 | ForEach-Object { "$($_.Path)|$($_.Hash)" } | Sort-Object)
}
function Assert-Signed([string]$Path) {
  $signature = Get-AuthenticodeSignature -LiteralPath $Path
  if ($signature.Status -ne 'Valid') { throw "Authenticode validation failed for ${Path}: $($signature.Status)" }
}
$before = Get-DocumentsSnapshot
Assert-Signed $msi
$installed = $false; $app = $null
try {
  if (-not $CiEphemeral) {
    $process = Start-Process msiexec.exe -ArgumentList @('/a', $msi, '/qn', "TARGETDIR=$install", '/l*v', (Join-Path $root 'extract.log')) -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) { throw "MSI administrative extraction failed: $($process.ExitCode)" }
    $exe = Get-ChildItem -LiteralPath $install -Recurse -File -Filter '*.exe' | Where-Object Name -NotMatch 'uninstall|setup' | Select-Object -First 1
    if (-not $exe) { throw 'Extracted application executable was not found.' }
    Assert-Signed $exe.FullName
    Write-Output "Dry acceptance passed. Administrative extraction retained at $root; no app was installed or launched."
  } else {
    $process = Start-Process msiexec.exe -ArgumentList @('/i', $msi, '/qn', "INSTALLDIR=$install", '/norestart', '/l*v', (Join-Path $root 'install.log')) -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) { throw "MSI installation failed: $($process.ExitCode)" }
    $installed = $true
    $exe = Get-ChildItem -LiteralPath $install -Recurse -File -Filter '*.exe' | Where-Object Name -NotMatch 'uninstall|setup' | Select-Object -First 1
    if (-not $exe) { throw 'Installed application executable was not found in the disposable root.' }
    Assert-Signed $exe.FullName
    $env:SMARTCAT_ACCEPTANCE_MODE = '1'; $env:SMARTCAT_ACCEPTANCE_ROOT = $data
    $app = Start-Process -FilePath $exe.FullName -PassThru -WindowStyle Hidden
    $ready = Join-Path $data 'app-ready.json'; $deadline = [DateTime]::UtcNow.AddSeconds(25)
    while (-not (Test-Path -LiteralPath $ready) -and [DateTime]::UtcNow -lt $deadline -and -not $app.HasExited) { Start-Sleep -Milliseconds 250 }
    if (-not (Test-Path -LiteralPath $ready)) { throw 'App did not reach hydrated main-window readiness in time.' }
    if (-not $app.HasExited) { Stop-Process -Id $app.Id -Force }
    Write-Output 'CI ephemeral install, launch-ready, and signature assertions passed; cleanup will uninstall and verify Documents.'
  }
} finally {
  if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force }
  if ($installed) {
    $uninstall = Start-Process msiexec.exe -ArgumentList @('/x', $msi, '/qn', '/norestart', '/l*v', (Join-Path $root 'uninstall.log')) -Wait -PassThru -WindowStyle Hidden
    if ($uninstall.ExitCode -ne 0) { throw "MSI uninstall failed: $($uninstall.ExitCode)" }
  }
  if (Compare-Object $before (Get-DocumentsSnapshot)) { throw 'User Documents changed during acceptance.' }
  if ($CiEphemeral -and (Test-Path -LiteralPath $root)) {
    $resolved = (Resolve-Path -LiteralPath $root).Path; $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if (-not $resolved.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase) -or (Split-Path $resolved -Leaf) -notlike 'smartcat-release-acceptance-*') { throw 'Refusing unsafe acceptance cleanup.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
  }
}
