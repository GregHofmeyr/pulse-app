# Pulse local test kit: LiveKit + pulse-app-server + the app, all on this machine (loopback only).
#   .\start.ps1           start everything, open the app
#   .\start.ps1 -Second   also open a second app on its own profile (log in as a second user)
param([switch]$Second)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

# Files extracted from a downloaded zip are marked "from the internet"; clear that for the kit.
Get-ChildItem -Recurse -File | Unblock-File

$data = Join-Path $PSScriptRoot 'data'
New-Item -ItemType Directory -Force $data | Out-Null

# A random LiveKit secret per install (never the dev one from the repo).
$secretFile = Join-Path $data 'livekit-secret.txt'
if (-not (Test-Path $secretFile)) {
    $bytes = New-Object byte[] 32
    [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    -join ($bytes | ForEach-Object { $_.ToString('x2') }) | Set-Content -NoNewline $secretFile
}
$secret = (Get-Content $secretFile -Raw).Trim()

@"
port: 7880
bind_addresses: ["127.0.0.1"]
rtc:
  tcp_port: 7881
  udp_port: 7882
  use_external_ip: false
  node_ip: 127.0.0.1
keys:
  pulse: $secret
webhook:
  api_key: pulse
  urls: ["http://127.0.0.1:7890/livekit/webhook"]
logging:
  level: info
"@ | Set-Content (Join-Path $data 'livekit.yaml')

$env:PULSE_DB_URL = 'sqlite://data/pulse.db'
$env:PULSE_BIND = '127.0.0.1:7890'
$env:PULSE_LIVEKIT_URL = 'ws://127.0.0.1:7880'
$env:PULSE_LIVEKIT_KEY = 'pulse'
$env:PULSE_LIVEKIT_SECRET = $secret

$firstRun = -not (Test-Path (Join-Path $data 'pulse.db'))

Write-Host 'Starting LiveKit...'
$lk = Start-Process -PassThru -WindowStyle Hidden '.\livekit-server.exe' `
    -ArgumentList '--config', 'data\livekit.yaml' `
    -RedirectStandardOutput 'data\livekit.log' -RedirectStandardError 'data\livekit.err.log'
Write-Host 'Starting server...'
$srv = Start-Process -PassThru -WindowStyle Hidden '.\pulse-app-server.exe' -ArgumentList 'serve' `
    -RedirectStandardOutput 'data\server.log' -RedirectStandardError 'data\server.err.log'

$up = $false
foreach ($i in 1..40) {
    try {
        if ((Invoke-RestMethod -TimeoutSec 1 'http://127.0.0.1:7890/health') -eq 'ok') { $up = $true; break }
    } catch { Start-Sleep -Milliseconds 250 }
}
if (-not $up) {
    Write-Host 'Server did not come up. See data\server.err.log' -ForegroundColor Red
    Stop-Process -Id $lk.Id, $srv.Id -ErrorAction SilentlyContinue
    Read-Host 'Press Enter to exit'
    exit 1
}

if ($firstRun) {
    Write-Host "`nFirst run: invite codes for two accounts (sign up with one each):" -ForegroundColor Cyan
    1..2 | ForEach-Object { & '.\pulse-app-server.exe' create-invite }
    Write-Host "Server URL in the app: http://127.0.0.1:7890`n" -ForegroundColor Cyan
}

Start-Process '.\pulse-app.exe'
if ($Second) {
    $env:PULSE_PROFILE = 'b'
    Start-Process '.\pulse-app.exe'
    Remove-Item Env:PULSE_PROFILE
}

Write-Host 'Pulse is running. Logs: data\ (server, LiveKit) and %LOCALAPPDATA%\app.pulse.client\logs (app).'
Read-Host 'Press Enter to stop the server and LiveKit'
Stop-Process -Id $srv.Id, $lk.Id -ErrorAction SilentlyContinue
