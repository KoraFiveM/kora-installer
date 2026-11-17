# The panel's Windows administrator worker (kora admin-worker / kora admin-step), as far as an unattended,
# already elevated runner can go: there is no permission prompt to answer here, so `--selftest` starts the
# worker directly instead of through `runas`. Everything else is real: the random pipe and its security
# descriptor, both peer checks (pid + same program file), the protocol, the allow-list, a real admin-step
# child and its result line, and the worker exiting after "bye". The prompt itself is in the manual checklist.
. "$PSScriptRoot/common.ps1"
$kora = Get-KoraExe

$r = Invoke-Native $kora @('admin-worker', '--selftest'); Show 'kora admin-worker --selftest' $r
Assert-That ($r.Code -eq 0) 'the self-test passes'
Assert-That ($r.Output -match 'passed both peer checks') 'the worker connected and both ends checked each other'
Assert-That ($r.Output -match 'refused install') 'a whole install is not run elevated'
Assert-That ($r.Output -match 'refused cmd\.exe') 'arbitrary programs are refused'
Assert-That ($r.Output -match 'refused admin-step run --step system.firewall --dir C:\\Windows') 'a folder without a Kora record (not the person''s) is refused'
Assert-That ($r.Output -match 'admin-step child ran and reported back') 'an admin-step child ran and sent its result'
Assert-That ($r.Output -match 'exited after bye') 'the worker exits when the panel is done'
Assert-That (-not (Get-Process -Name kora -ErrorAction SilentlyContinue | Where-Object { $_.CommandLine -match 'admin-worker --pipe' })) 'no worker is left running'

# A worker with nobody at the other end gives up instead of waiting forever.
$pipe = '\\.\pipe\kora-admin-1-' + ('0' * 32)
$sw = [Diagnostics.Stopwatch]::StartNew()
$r = Invoke-Native $kora @('admin-worker', '--pipe', $pipe, '--parent', '1'); Show 'kora admin-worker (no panel)' $r
Assert-That ($r.Code -ne 0) 'a worker without its panel fails'
Assert-That ($sw.Elapsed.TotalSeconds -lt 60) "and does so quickly ($([int]$sw.Elapsed.TotalSeconds) s)"

# Arguments that are not the panel's are refused before anything happens.
$r = Invoke-Native $kora @('admin-worker', '--pipe', '\\otherpc\pipe\x', '--parent', '1'); Show 'kora admin-worker (foreign pipe)' $r
Assert-That ($r.Code -ne 0 -and $r.Output -match 'needs --pipe and --parent from Kora Panel') 'a pipe name that is not the panel''s is refused'

# An admin step on a folder without a Kora record does nothing and says why (and still prints its result line).
$r = Invoke-Native $kora @('admin-step', 'run', '--step', 'system.firewall', '--dir', 'C:\kora-smoke\no such folder'); Show 'kora admin-step (no record)' $r
Assert-That ($r.Code -eq 1) 'admin-step without a record exits 1'
Assert-That ($r.Output -match 'KORA-ADMIN-RESULT \{') 'the result line is still printed for the panel'

exit 0
