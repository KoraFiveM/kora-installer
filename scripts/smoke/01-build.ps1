. "$PSScriptRoot/common.ps1"
$kora = Get-KoraExe
$v = Invoke-Native $kora @('--version'); Show 'kora --version' $v
Assert-That ($v.Code -eq 0 -and $v.Output -match '^kora \d+\.\d+\.\d+') "kora.exe --version prints a version ($($v.Output.Trim()))"

# No TTY here (stdin/stdout redirected): the wizard must refuse with the friendly message, exit 2.
$r = Invoke-Native $kora @('install'); Show 'kora install (no TTY)' $r
Assert-That ($r.Code -eq 2) 'kora install without a terminal exits 2'
Assert-That ($r.Output -match 'needs an interactive terminal') 'the error explains that a terminal is needed'

$r = Invoke-Native $kora @('status'); Show 'kora status (empty)' $r
Assert-That ($r.Code -eq 0) 'kora status works with no servers registered'
