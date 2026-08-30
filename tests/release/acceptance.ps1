param([Parameter(Mandatory=$true)][string]$MsiPath, [switch]$CiEphemeral)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Windows acceptance must run on Windows.' }
$msi = (Resolve-Path -LiteralPath $MsiPath).Path
if ([IO.Path]::GetExtension($msi) -ne '.msi') { throw 'Acceptance requires an MSI artifact.' }
if ($CiEphemeral -and ($env:CI -ne 'true' -or $env:GITHUB_ACTIONS -ne 'true')) { throw '-CiEphemeral is allowed only on a GitHub Actions ephemeral runner.' }
$appDataOwned = $false; $installedByThisRun = $false; $rootOwned = $false; $app = $null
$runnerLocalData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
$expectedAppData = [IO.Path]::GetFullPath((Join-Path $runnerLocalData 'com.smartcat.translate'))
$root = Join-Path ([IO.Path]::GetTempPath()) ('smartcat-release-acceptance-' + [guid]::NewGuid())
$install = Join-Path $root 'app'; $data = Join-Path $root 'test-data'
New-Item -ItemType Directory -Path $install,$data -Force | Out-Null
$rootOwned = $true
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
try {
  if (-not $CiEphemeral) {
    $process = Start-Process msiexec.exe -ArgumentList @('/a', $msi, '/qn', "TARGETDIR=$install", '/l*v', (Join-Path $root 'extract.log')) -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) { throw "MSI administrative extraction failed: $($process.ExitCode)" }
    $exe = Get-ChildItem -LiteralPath $install -Recurse -File -Filter '*.exe' | Where-Object Name -NotMatch 'uninstall|setup' | Select-Object -First 1
    if (-not $exe) { throw 'Extracted application executable was not found.' }
    $msiStatus = (Get-AuthenticodeSignature -LiteralPath $msi).Status
    $exeStatus = (Get-AuthenticodeSignature -LiteralPath $exe.FullName).Status
    Write-Output "Dry acceptance passed. Administrative extraction retained at $root; no app was installed or launched. Signature status: MSI=$msiStatus app=$exeStatus."
  } else {
    Assert-Signed $msi
    if (Test-Path -LiteralPath $expectedAppData) { throw 'GitHub runner app-data path was not clean before acceptance.' }
    $credentialTarget = 'local-data-key.com.smartcat.translate'
    $credentialListing = cmdkey.exe "/list:$credentialTarget" 2>$null | Out-String
    if ($credentialListing -match [Regex]::Escape($credentialTarget)) { throw 'GitHub runner Credential Manager already contains the SmartCAT key.' }
    $existingInstall = @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*') |
      ForEach-Object { Get-ItemProperty -Path $_ -ErrorAction SilentlyContinue } | Where-Object DisplayName -EQ 'SmartCAT Translate'
    if ($existingInstall) { throw 'GitHub runner already has SmartCAT Translate installed.' }
    $process = Start-Process msiexec.exe -ArgumentList @('/i', $msi, '/qn', "INSTALLDIR=$install", '/norestart', '/l*v', (Join-Path $root 'install.log')) -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) { throw "MSI installation failed: $($process.ExitCode)" }
    $installedByThisRun = $true
    $exe = Get-ChildItem -LiteralPath $install -Recurse -File -Filter '*.exe' | Where-Object Name -NotMatch 'uninstall|setup' | Select-Object -First 1
    if (-not $exe) { throw 'Installed application executable was not found in the disposable root.' }
    Assert-Signed $exe.FullName
    $app = Start-Process -FilePath $exe.FullName -PassThru -WindowStyle Hidden
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
      Start-Sleep -Milliseconds 250; $app.Refresh()
      if (-not $appDataOwned -and (Test-Path -LiteralPath $expectedAppData)) { $appDataOwned = $true }
    } while (-not $app.HasExited -and $app.MainWindowHandle -eq 0 -and [DateTime]::UtcNow -lt $deadline)
    if ($app.HasExited -or $app.MainWindowHandle -eq 0) { throw 'App failed to open its main window through the real Credential Manager/default app-data path.' }
    $stableUntil = [DateTime]::UtcNow.AddSeconds(10)
    while ([DateTime]::UtcNow -lt $stableUntil) {
      Start-Sleep -Milliseconds 250; $app.Refresh()
      if (-not $appDataOwned -and (Test-Path -LiteralPath $expectedAppData)) { $appDataOwned = $true }
      if ($app.HasExited) { throw 'App exited during the secure-store readiness interval.' }
    }
    if (-not $app.HasExited) { Stop-Process -Id $app.Id -Force }
    Write-Output 'CI ephemeral install, main-window stability, real Credential Manager, default app-data, and signature assertions passed.'
  }
} finally {
  if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force }
  if ($installedByThisRun) {
    $uninstall = Start-Process msiexec.exe -ArgumentList @('/x', $msi, '/qn', '/norestart', '/l*v', (Join-Path $root 'uninstall.log')) -Wait -PassThru -WindowStyle Hidden
    if ($uninstall.ExitCode -ne 0) { throw "MSI uninstall failed: $($uninstall.ExitCode)" }
  }
  if ($appDataOwned -and (Test-Path -LiteralPath $expectedAppData)) {
    $trustedParent = [IO.Path]::GetFullPath($runnerLocalData).TrimEnd([IO.Path]::DirectorySeparatorChar)
    $resolvedData = [IO.Path]::GetFullPath($expectedAppData)
    if ((Split-Path $resolvedData -Parent) -ne $trustedParent -or (Split-Path $resolvedData -Leaf) -ne 'com.smartcat.translate') { throw 'Refusing cleanup outside exact runner SmartCAT app-data path.' }
    if ((Get-Item -LiteralPath $resolvedData -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refusing to follow an app-data reparse point.' }
    Remove-Item -LiteralPath $resolvedData -Recurse -Force
  }
  if (Compare-Object $before (Get-DocumentsSnapshot)) { throw 'User Documents changed during acceptance.' }
  if ($rootOwned -and $CiEphemeral -and (Test-Path -LiteralPath $root)) {
    $resolved = (Resolve-Path -LiteralPath $root).Path; $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if (-not $resolved.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase) -or (Split-Path $resolved -Leaf) -notlike 'smartcat-release-acceptance-*') { throw 'Refusing unsafe acceptance cleanup.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
  }
}
