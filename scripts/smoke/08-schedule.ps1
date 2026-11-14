# `kora schedule show|set|off` against the real Task Scheduler: as the installing user (interactive
# token, no password) and as SYSTEM, then removed again. Runs after step 4 (any registered server works).
. "$PSScriptRoot/common.ps1"
$kora = Get-KoraExe
$srv = 'C:\kora-smoke\sched srv'
$task = 'Kora\Update sched'
$env:KORA_CATALOG = (Resolve-Path "$PWD/tests/fixtures/smoke-catalog.json").Path
$env:CFX_LICENSE_KEY = 'cfxk_FAKEFAKEFAKEFAKEFAKE_smoke'

function Get-TaskXml { $o = (& schtasks.exe /Query /TN $task /XML 2>&1 | Out-String); if ($LASTEXITCODE -ne 0) { $null } else { $o } }

$r = Invoke-Native $kora @('install', '--name', 'sched', '--dir', $srv, '--game-port', '31130', '--txadmin-port', '41130', '--db', 'skip', '-y'); Show 'kora install sched' $r
Assert-That ($r.Code -eq 0) 'install of the schedule test server exits 0'

$r = Invoke-Native $kora @('schedule', 'show', 'sched'); Show 'schedule show (nothing yet)' $r
Assert-That ($r.Output -match 'no automatic updates') 'show says nothing is scheduled'

# --- as the installing user (default) ---------------------------------------------------
$r = Invoke-Native $kora @('schedule', 'set', 'sched', '--daily', '05:30'); Show 'schedule set --daily 05:30' $r
Assert-That ($r.Code -eq 0) 'schedule set exits 0'
Assert-That ($r.Output -match 'Every day at 05:30, as you') 'output is plain language'
$x = Get-TaskXml
Assert-That ($null -ne $x) 'the scheduled task exists'
Assert-That ($x -match '05:30:00') 'trigger time is 05:30'
Assert-That ($x -match 'InteractiveToken') 'runs "only when logged on" (no stored password)'
Assert-That ($x -match 'StartWhenAvailable>true') 'a missed run happens at the next opportunity'
Assert-That ($x -notmatch 'S-1-5-18') 'not SYSTEM'
Assert-That ($x -match 'update --auto') 'the task runs kora update --auto'
Assert-That (-not (Test-Path "$srv\.kora\update-task.xml")) 'the temporary task XML is cleaned up'
Assert-That (Test-Path "$srv\.kora\schedule.json") 'the setting is stored in the server folder'
$r = Invoke-Native $kora @('schedule', 'show', 'sched'); Show 'schedule show' $r
Assert-That ($r.Output -match 'Every day at 05:30, as you') 'show reads it back'

# --- change to weekly + SYSTEM ----------------------------------------------------------
$r = Invoke-Native $kora @('schedule', 'set', 'sched', '--weekly', 'sun', '03:15', '--as-system'); Show 'schedule set --weekly sun 03:15 --as-system' $r
Assert-That ($r.Code -eq 0) 'schedule set (system) exits 0'
$x = Get-TaskXml
Assert-That ($x -match '03:15:00' -and $x -match 'Sunday') 'weekly Sunday 03:15'
Assert-That ($x -match 'S-1-5-18|SYSTEM') 'runs as SYSTEM'
$r = Invoke-Native $kora @('schedule', 'show', 'sched'); Show 'schedule show' $r
Assert-That ($r.Output -match 'system account' -and $r.Output -match 'private') 'show explains the private-fork limit'

# --- bad input is refused, nothing changes ----------------------------------------------
$r = Invoke-Native $kora @('schedule', 'set', 'sched', '--daily', '25:00'); Show 'schedule set 25:00' $r
Assert-That ($r.Code -ne 0) 'an invalid time is refused'
Assert-That ((Get-TaskXml) -match '03:15:00') 'the task is unchanged'

# --- off --------------------------------------------------------------------------------
$r = Invoke-Native $kora @('schedule', 'off', 'sched'); Show 'schedule off' $r
Assert-That ($r.Code -eq 0) 'schedule off exits 0'
Assert-That ($null -eq (Get-TaskXml)) 'the task is gone'
Assert-That (-not (Test-Path "$srv\.kora\schedule.json")) 'the stored setting is gone'
$r = Invoke-Native $kora @('schedule', 'off', 'sched'); Assert-That ($r.Code -eq 0) 'off twice is fine'

$r = Invoke-Native $kora @('delete', 'sched', '-y'); Show 'kora delete sched' $r
Assert-That ($r.Code -eq 0) 'cleanup'
