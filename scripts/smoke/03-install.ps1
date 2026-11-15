. "$PSScriptRoot/common.ps1"
$kora = Get-KoraExe
$srv = 'C:\kora-smoke\srv'
$fakeKey = 'cfxk_FAKEFAKEFAKEFAKEFAKE_smoke'
$env:KORA_CATALOG = (Resolve-Path "$PWD/tests/fixtures/smoke-catalog.json").Path
$env:CFX_LICENSE_KEY = $fakeKey

$r = Invoke-Native $kora @('install', '--name', 'smoke', '--dir', $srv, '--game-port', '31120', '--txadmin-port', '41120', '--db', 'skip', '-y')
Show 'kora install (production, no service)' $r
Assert-That ($r.Code -eq 0) 'kora install exits 0'
Assert-That ($r.Output -notmatch [regex]::Escape($fakeKey)) 'the license key is never printed'

foreach ($lib in 'kora_lib', 'kora_sql', 'kora_core') {
    Assert-That (Test-Path -LiteralPath "$srv\resources\[pre]\$lib") "resource $lib cloned under resources\[pre]"
}
Assert-That (Test-Path -LiteralPath "$srv\resources\[standalone]\kora_theme") 'the kora_theme dependency (pulled in by kora_lib) is present'
Assert-That (Test-Path -LiteralPath "$srv\resources\[cfx-default]") 'the cfx-default dependency is present'
$fx = Get-ChildItem "$srv\artifacts" -Recurse -Filter FXServer.exe | Select-Object -First 1
Assert-That ($null -ne $fx) 'the Windows FXServer.exe artifact was extracted'
foreach ($f in 'server.cfg', 'permissions.cfg', 'secrets.cfg', 'kora-server.json', '.kora\created.json', 'txData\default') {
    Assert-That (Test-Path "$srv\$f") "$f exists"
}
$cfg = Get-Content "$srv\server.cfg" -Raw
Assert-That ($cfg -match 'exec secrets\.cfg') 'server.cfg execs secrets.cfg'
Assert-That ($cfg -match 'endpoint_add_tcp "0\.0\.0\.0:31120"') 'server.cfg uses the game port 31120'
Assert-That ($cfg -notmatch [regex]::Escape($fakeKey)) 'server.cfg holds no license key'
Assert-That ($cfg -match 'ensure \[pre\]') 'server.cfg ensures the [pre] folder (folders, not single resources)'
$sec = Get-Content "$srv\secrets.cfg" -Raw
Assert-That ($sec -match [regex]::Escape($fakeKey)) 'secrets.cfg holds the license key'
$leak = Get-ChildItem $srv -Recurse -File -Force -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notmatch '\\(resources|artifacts)\\' -and $_.Name -ne 'secrets.cfg' } | Select-String -SimpleMatch $fakeKey -List
Assert-That (-not $leak) 'the license key is written nowhere else (state, request.json, logs)'
$txa = Get-Content "$srv\txData\default\config.json" -Raw -ErrorAction SilentlyContinue
if ($txa) { Assert-That ($txa -notmatch [regex]::Escape($fakeKey)) 'txAdmin config holds no license key' }

$acl = Get-Acl-Text "$srv\secrets.cfg"; Write-Host $acl
Assert-That ($acl -notmatch '(?i)\\Users:|Everyone|Authenticated Users|BUILTIN\\Users') 'secrets.cfg ACL does not grant Users/Everyone/Authenticated Users'
Assert-That ($acl -match [regex]::Escape($env:USERNAME)) 'secrets.cfg ACL names the installing user'
$readOk = $false; try { [void][IO.File]::ReadAllText("$srv\secrets.cfg"); $readOk = $true } catch {}
Assert-That $readOk 'the installing user can still read secrets.cfg'

$reg = Get-Content "$env:LOCALAPPDATA\Kora\servers.json" -Raw | ConvertFrom-Json
Assert-That (@($reg.servers | Where-Object { $_.name -eq 'smoke' -and $_.ports.game -eq 31120 -and $_.ports.txadmin -eq 41120 }).Count -eq 1) 'registry (servers.json) has the smoke server with its ports'

$st = Invoke-Native $kora @('status'); Show 'kora status' $st
Assert-That ($st.Code -eq 0 -and $st.Output -match 'smoke\s+prod\s+31120\s+41120\s+stopped') 'kora status lists the server as stopped'

# Re-running the same install resumes cleanly (idempotent).
$r = Invoke-Native $kora @('install', '--name', 'smoke', '--dir', $srv, '--game-port', '31120', '--txadmin-port', '41120', '--db', 'skip', '-y')
Show 'kora install (resume)' $r
Assert-That ($r.Code -eq 0) 'running the same install again succeeds'

exit 0
