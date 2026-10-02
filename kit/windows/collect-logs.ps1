# Zip every Pulse log (app, all profiles, server, LiveKit) to the Desktop for sharing.
$ErrorActionPreference = 'Stop'
$stage = Join-Path $env:TEMP "pulse-logs-$(Get-Date -Format yyyyMMdd-HHmmss)"
New-Item -ItemType Directory -Force $stage | Out-Null

$appLogs = Join-Path $env:LOCALAPPDATA 'app.pulse.client\logs'
if (Test-Path $appLogs) { Copy-Item -Recurse $appLogs (Join-Path $stage 'app') }
$kitLogs = Join-Path $PSScriptRoot 'data'
if (Test-Path $kitLogs) {
    New-Item -ItemType Directory -Force (Join-Path $stage 'kit') | Out-Null
    Copy-Item (Join-Path $kitLogs '*.log') (Join-Path $stage 'kit') -ErrorAction SilentlyContinue
}

$zip = Join-Path ([Environment]::GetFolderPath('Desktop')) "$(Split-Path $stage -Leaf).zip"
Compress-Archive -Path "$stage\*" -DestinationPath $zip
Remove-Item -Recurse $stage
Write-Host "Logs saved to $zip"
