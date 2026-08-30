param([Parameter(Mandatory=$true)][string]$MsiPath, [switch]$RunShortSmoke)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Windows acceptance must run on Windows.' }
$msi = (Resolve-Path -LiteralPath $MsiPath).Path
if ([IO.Path]::GetExtension($msi) -ne '.msi') { throw 'Acceptance requires an MSI artifact.' }
$root = Join-Path ([IO.Path]::GetTempPath()) ('smartcat-release-acceptance-' + [guid]::NewGuid())
$install = Join-Path $root 'administrative-image'
$data = Join-Path $root 'test-data'
New-Item -ItemType Directory -Path $install,$data -Force | Out-Null
$documents = [Environment]::GetFolderPath('MyDocuments')
$before = if (Test-Path -LiteralPath $documents) { Get-ChildItem -LiteralPath $documents -File -Recurse -ErrorAction SilentlyContinue | Get-FileHash -Algorithm SHA256 } else { @() }
$arguments = @('/a', $msi, '/qn', "TARGETDIR=$install", '/l*v', (Join-Path $root 'msi.log'))
$process = Start-Process msiexec.exe -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
if ($process.ExitCode -ne 0) { throw "MSI administrative extraction failed: $($process.ExitCode)" }
$exe = Get-ChildItem -LiteralPath $install -Recurse -File -Filter '*.exe' | Where-Object Name -NotMatch 'uninstall|setup' | Select-Object -First 1
if (-not $exe) { throw 'Installed application executable was not found.' }
Get-AuthenticodeSignature -LiteralPath $exe.FullName | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'signature.json')
if ($RunShortSmoke) {
  $env:LOCALAPPDATA = Join-Path $data 'LocalAppData'; $env:APPDATA = Join-Path $data 'RoamingAppData'
  New-Item -ItemType Directory -Path $env:LOCALAPPDATA,$env:APPDATA -Force | Out-Null
  $app = Start-Process -FilePath $exe.FullName -PassThru -WindowStyle Hidden
  Write-Host 'Perform the short checklist in docs/release-smoke-checklist.md, then press Enter.'
  Read-Host | Out-Null
  if (-not $app.HasExited) { Stop-Process -Id $app.Id }
}
$after = if (Test-Path -LiteralPath $documents) { Get-ChildItem -LiteralPath $documents -File -Recurse -ErrorAction SilentlyContinue | Get-FileHash -Algorithm SHA256 } else { @() }
if (Compare-Object ($before | ForEach-Object { "$($_.Path)|$($_.Hash)" }) ($after | ForEach-Object { "$($_.Path)|$($_.Hash)" })) { throw 'User Documents changed during acceptance.' }
Write-Output "Acceptance evidence retained at $root. Only the administrative image and disposable test-data root were used; user documents were unchanged."
