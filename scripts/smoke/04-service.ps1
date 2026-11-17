. "$PSScriptRoot/common.ps1"
$kora = Get-KoraExe
$srv = 'C:\kora-smoke\srv'
$svc = 'kora-smoke'
$fakeKey = 'cfxk_FAKEFAKEFAKEFAKEFAKE_smoke'
$env:KORA_CATALOG = (Resolve-Path "$PWD/tests/fixtures/smoke-catalog.json").Path
$env:CFX_LICENSE_KEY = $fakeKey

function Get-SvcState { $o = (& sc.exe query $svc | Out-String); if ($o -match 'STATE\s+:\s+\d+\s+(\w+)') { $Matches[1] } elseif ($o -match '1060') { 'GONE' } else { "UNKNOWN: $o" } }
function Wait-SvcState([string]$Want, [int]$Seconds = 60) {
    $end = (Get-Date).AddSeconds($Seconds)
    do { $s = Get-SvcState; if ($s -eq $Want) { return $s }; Start-Sleep -Seconds 2 } while ((Get-Date) -lt $end)
    return $s
}
function Show-ServerLogs {
    Write-Host '--- FXServer / txAdmin logs (tails) ---'
    if (Test-Path "$srv\.kora\service-host.log") { Write-Host '== service-host.log'; Get-Content "$srv\.kora\service-host.log" -Tail 60 }
    foreach ($p in "$srv\txData\default\logs", "$srv\txData\logs", "$srv\logs") {
        if (Test-Path $p) { Get-ChildItem $p -Recurse -File | Sort-Object LastWriteTime | Select-Object -Last 4 | ForEach-Object { Write-Host "== $($_.FullName)"; Get-Content $_.FullName -Tail 40 } }
    }
    Get-ChildItem $srv -Recurse -File -Include *.log -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notmatch '\\(resources|artifacts)\\' } | Select-Object -Last 4 | ForEach-Object { Write-Host "== $($_.FullName)"; Get-Content $_.FullName -Tail 40 }
    Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Service Control Manager' } -MaxEvents 10 -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "SCM: $($_.TimeCreated) $($_.Message)" }
}

Assert-That (Test-Path $srv) 'the earlier step-3 server exists (this step starts from it)'
# --- kora delete on the no-service server: folder + registry entry gone ---------------------
$r = Invoke-Native $kora @('delete', 'smoke', '-y'); Show 'kora delete smoke -y' $r
Assert-That ($r.Code -eq 0) 'kora delete exits 0'
Assert-That (-not (Test-Path $srv)) 'kora delete removed the server folder (no file-lock leftovers)'
$reg = Get-Content "$env:LOCALAPPDATA\Kora\servers.json" -Raw -ErrorAction SilentlyContinue
Assert-That (-not $reg -or $reg -notmatch '"smoke"') 'kora delete removed the registry entry'

# --- fresh install with the service + firewall ---------------------------------------------
$r = Invoke-Native $kora @('install', '--name', 'smoke', '--dir', $srv, '--game-port', '31120', '--txadmin-port', '41120', '--db', 'skip', '--service', '--firewall', '--txadmin-public', '-y')
Show 'kora install --service --firewall' $r
Assert-That ($r.Code -eq 0) 'kora install --service --firewall exits 0'
Assert-That ((Wait-SvcState 'STOPPED' 10) -eq 'STOPPED') "service $svc is registered and stopped"
$qc = (& sc.exe qc $svc | Out-String); Write-Host $qc
Assert-That ($qc -match 'AUTO_START') 'service start type is automatic'
Assert-That ($qc -match 'NT SERVICE\\kora-smoke') 'service runs as the NT SERVICE virtual account'
Assert-That ($qc -match 'service-host') 'service command line is `kora service-host`'
$bin = ($qc -split "`n" | Where-Object { $_ -match 'BINARY_PATH_NAME' }) -replace '^\s*BINARY_PATH_NAME\s*:\s*', ''
Assert-That ($bin -match [regex]::Escape($env:ProgramFiles)) 'service binary is the Program Files copy (not a user-writable path)'

$fw = (& netsh advfirewall firewall show rule name=all | Out-String)
Assert-That ($fw -match 'Kora smoke game 31120 TCP' -and $fw -match 'Kora smoke game 31120 UDP') 'firewall: game port rules TCP+UDP added'
Assert-That ($fw -match 'Kora smoke txadmin 41120 TCP') 'firewall: txAdmin rule added (--txadmin-public)'

$acl = Get-Acl-Text "$srv\secrets.cfg"; Write-Host $acl
Assert-That ($acl -notmatch '(?i)BUILTIN\\Users|Everyone|Authenticated Users') 'secrets.cfg still not readable by Users/Everyone with the service installed'
Assert-That ($acl -match 'NT SERVICE\\kora-smoke') 'secrets.cfg is readable by the service account (otherwise FXServer cannot read its license key)'

# --- start / restart / stop through kora ----------------------------------------------------
$r = Invoke-Native $kora @('start', 'smoke'); Show 'kora start smoke' $r
$state = Wait-SvcState 'RUNNING' 60
if ($state -ne 'RUNNING') { Show-ServerLogs }
Assert-That ($r.Code -eq 0) 'kora start exits 0'
Assert-That ($state -eq 'RUNNING') "sc query: $svc is RUNNING after kora start (was $state)"

$up = $false
$end = (Get-Date).AddSeconds(150)
while ((Get-Date) -lt $end -and -not $up) {
    try { $resp = Invoke-WebRequest -UseBasicParsing -Uri 'http://127.0.0.1:41120/' -TimeoutSec 5 -MaximumRedirection 0 -SkipHttpErrorCheck; $up = $true; Write-Host "txAdmin answered HTTP $($resp.StatusCode)" } catch { Start-Sleep -Seconds 3 }
}
Show-ServerLogs
Assert-That $up 'txAdmin answers on 127.0.0.1:41120 (fake key)'
$fx = @(Get-Process FXServer -ErrorAction SilentlyContinue)
Assert-That ($fx.Count -ge 1) 'an FXServer.exe process is running'
$st = Invoke-Native $kora @('status'); Show 'kora status' $st
Assert-That ($st.Output -match 'smoke\s+prod\s+31120\s+41120\s+running') 'kora status shows running'

# The service keeps only ordinary-user privileges (no SeImpersonatePrivilege) and still serves txAdmin (above).
$q = (& sc.exe qprivs $svc 2>&1 | Out-String); Write-Host $q
Assert-That ($q -match 'SeChangeNotifyPrivilege' -and $q -notmatch 'SeImpersonatePrivilege') 'sc qprivs: only ordinary-user privileges'
# The installing user may start and stop this one service (what the unelevated update task needs).
$me = ([System.Security.Principal.WindowsIdentity]::GetCurrent()).User.Value
$sd = (& sc.exe sdshow $svc 2>&1 | Out-String); Write-Host $sd
# sc prints the built-in Administrator account (the CI runner's) as LA instead of its SID.
$meShown = if ($me -match '-500$') { "(?:$([regex]::Escape($me))|LA)" } else { [regex]::Escape($me) }
Assert-That ($sd -match "\(A;;CCLCSWRPWPDTLOCRRC;;;$meShown\)") 'sc sdshow: an entry for the installing user (start and stop, nothing more)'

Start-Sleep -Seconds 15
Assert-That ((Get-SvcState) -eq 'RUNNING') 'the service is STILL running 15 s after start (FXServer did not die and get restarted by the SCM)'
$r = Invoke-Native $kora @('restart', 'smoke'); Show 'kora restart smoke' $r
Assert-That ($r.Code -eq 0) 'kora restart exits 0'
Assert-That ((Wait-SvcState 'RUNNING' 60) -eq 'RUNNING') 'RUNNING again after restart'

# Plain Windows tools too (what an admin would type): sc stop -> STOPPED, sc start -> RUNNING.
& sc.exe stop $svc | Out-Host
Assert-That ((Wait-SvcState 'STOPPED' 60) -eq 'STOPPED') 'sc stop (not via kora) reaches STOPPED, so the SCM stop control is handled'
& sc.exe start $svc | Out-Host
Assert-That ((Wait-SvcState 'RUNNING' 60) -eq 'RUNNING') 'sc start (not via kora) reaches RUNNING'
$r = Invoke-Native $kora @('stop', 'smoke'); Show 'kora stop smoke' $r
Assert-That ($r.Code -eq 0) 'kora stop exits 0'
Assert-That ((Wait-SvcState 'STOPPED' 60) -eq 'STOPPED') 'sc query: STOPPED after kora stop'
Start-Sleep -Seconds 3
Assert-That (@(Get-Process FXServer -ErrorAction SilentlyContinue).Count -eq 0) 'no FXServer.exe left behind after stop'

# --- uninstall removes everything ------------------------------------------------------------
$r = Invoke-Native $kora @('uninstall', '--yes', '--delete-files'); Show 'kora uninstall --yes --delete-files' $r
Assert-That ($r.Code -eq 0) 'kora uninstall exits 0'
Assert-That ((Get-SvcState) -eq 'GONE') 'the service is gone'
$fw = (& netsh advfirewall firewall show rule name=all | Out-String)
Assert-That ($fw -notmatch 'Kora smoke') 'all Kora firewall rules are gone'
Assert-That (-not (Test-Path $srv)) 'the server folder is gone'
$reg = Get-Content "$env:LOCALAPPDATA\Kora\servers.json" -Raw -ErrorAction SilentlyContinue
Assert-That (-not $reg -or $reg -notmatch '"smoke"') 'the registry entry is gone'
Assert-That (-not (Test-Path "$env:ProgramFiles\Kora\bin\*\kora.exe")) 'the Program Files kora copy is gone'
$t = (& schtasks.exe /Query /FO LIST 2>&1 | Out-String)
Assert-That ($t -notmatch 'Kora.*smoke') 'no Kora scheduled task is left'

exit 0
