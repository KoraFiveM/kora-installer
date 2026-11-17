. "$PSScriptRoot/common.ps1"
$kora = Get-KoraExe
$v = Invoke-Native $kora @('--version'); Show 'kora --version' $v
Assert-That ($v.Code -eq 0 -and $v.Output -match '^kora \d+\.\d+\.\d+') "kora.exe --version prints a version ($($v.Output.Trim()))"

# A removed flag still parses and explains itself (a bare `kora install` now opens the panel, so it is not run here).
$r = Invoke-Native $kora @('install', '--code', 'KORA-0000-0000'); Show 'kora install --code' $r
Assert-That ($r.Code -eq 2) 'kora install --code exits 2'
Assert-That ($r.Output -match 'setup codes were removed') 'the error explains that setup codes are gone'

$r = Invoke-Native $kora @('status'); Show 'kora status (empty)' $r
Assert-That ($r.Code -eq 0) 'kora status works with no servers registered'

exit 0
