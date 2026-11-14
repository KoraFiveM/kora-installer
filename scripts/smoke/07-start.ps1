# Install with --start: the installer itself starts FXServer (no service), waits for txAdmin, and the
# server keeps running after the installer exits. Then stop/start/delete through kora.
. "$PSScriptRoot/common.ps1"
$kora = Get-KoraExe
$srv = 'C:\kora-smoke\srv-start'
$name = 'smoke-start'
$env:KORA_CATALOG = (Resolve-Path "$PWD/tests/fixtures/smoke-catalog.json").Path
$env:CFX_LICENSE_KEY = 'cfxk_FAKEFAKEFAKEFAKEFAKE_smoke'   # a fake key: txAdmin comes up, the game server is refused

function Test-TxAdmin {
    try { $null = Invoke-WebRequest -UseBasicParsing -Uri 'http://127.0.0.1:41120/' -TimeoutSec 5 -MaximumRedirection 0 -SkipHttpErrorCheck; $true } catch { $false }
}
function Show-StartLogs {
    foreach ($f in "$srv\.kora\server.log") { if (Test-Path $f) { Write-Host "== $f"; Get-Content $f -Tail 60 } }
}

$r = Invoke-Native $kora @('install', '--name', $name, '--dir', $srv, '--game-port', '31120', '--txadmin-port', '41120', '--db', 'skip', '--start', '-y')
Show 'kora install --start' $r
if ($r.Code -ne 0) { Show-StartLogs }
Assert-That ($r.Code -eq 0) 'kora install --start exits 0'
Assert-That ($r.Output -match 'Start your server \(launch\)') 'the install ran the launch step'
Assert-That ($r.Output -match 'txAdmin is up on http://localhost:41120') 'the launch step reports txAdmin up on its port'
Assert-That ($r.Output -match 'licence key was rejected') 'a fake licence key is reported in plain words (not claimed as success)'
Assert-That ($r.Output -notmatch 'password ''?[A-Za-z0-9]{20}') 'the generated txAdmin password is not printed'

# The installer has exited: the server must still be running.
Assert-That (Test-Path "$srv\.kora\server.pid") 'the server pid was recorded in .kora\server.pid'
$pidNow = [int]((Get-Content "$srv\.kora\server.pid" -Raw).Trim())
$p = Get-Process -Id $pidNow -ErrorAction SilentlyContinue
Assert-That ($null -ne $p -and $p.ProcessName -match 'FXServer') "FXServer.exe (pid $pidNow) survives the installer exiting"
Assert-That (Test-TxAdmin) 'txAdmin still answers on 127.0.0.1:41120 after the installer exited'
Assert-That ((Get-Item "$srv\.kora\server.log").Length -gt 0) 'FXServer output went to .kora\server.log'

$st = Invoke-Native $kora @('status'); Show 'kora status' $st
Assert-That ($st.Output -match "$name\s+prod\s+31120\s+41120\s+running") 'kora status shows the detached server as running'
Assert-That ($st.Output -match 'txAdmin up') 'kora status shows the txAdmin port state'

# kora stop ends the whole process tree.
$r = Invoke-Native $kora @('stop', $name); Show 'kora stop' $r
Assert-That ($r.Code -eq 0) 'kora stop exits 0'
Start-Sleep -Seconds 3
Assert-That ($null -eq (Get-Process -Id $pidNow -ErrorAction SilentlyContinue)) 'the FXServer process is gone after kora stop'
Assert-That (@(Get-Process FXServer -ErrorAction SilentlyContinue).Count -eq 0) 'no FXServer.exe is left behind'
Assert-That (-not (Test-TxAdmin)) 'txAdmin no longer answers'
Assert-That (-not (Test-Path "$srv\.kora\server.pid")) 'the pid file is removed'

# kora start reuses the same logic (no service registered -> detached again).
$r = Invoke-Native $kora @('start', $name); Show 'kora start' $r
Assert-That ($r.Output -match 'Starting txAdmin') 'kora start shows progress'
Assert-That (Test-TxAdmin) 'txAdmin answers again after kora start'
$pid2 = [int]((Get-Content "$srv\.kora\server.pid" -Raw).Trim())
Assert-That ($null -ne (Get-Process -Id $pid2 -ErrorAction SilentlyContinue)) 'the restarted FXServer is alive'

# Deleting a running server stops it first (no locked files), then removes everything.
$r = Invoke-Native $kora @('delete', $name, '-y'); Show 'kora delete (server running)' $r
Assert-That ($r.Code -eq 0) 'kora delete exits 0 while the server runs'
Assert-That ($null -eq (Get-Process -Id $pid2 -ErrorAction SilentlyContinue)) 'delete stopped the running FXServer'
Assert-That (-not (Test-Path $srv)) 'the server folder is gone'
$reg = Get-Content "$env:LOCALAPPDATA\Kora\servers.json" -Raw -ErrorAction SilentlyContinue
Assert-That (-not $reg -or $reg -notmatch [regex]::Escape($name)) 'the registry entry is gone'
exit 0
