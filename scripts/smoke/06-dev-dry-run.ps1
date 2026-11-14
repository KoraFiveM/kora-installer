. "$PSScriptRoot/common.ps1"
$kora = Get-KoraExe
$env:KORA_CATALOG = (Resolve-Path "$PWD/tests/fixtures/smoke-catalog.json").Path
$r = Invoke-Native $kora @('install', '--mode', 'dev', '--github-owner', 'smoke-owner', '--dir', 'C:\kora-smoke\dev', '--db', 'skip', '--dry-run', '-y') @{ CFX_LICENSE_KEY = 'cfxk_FAKE' }
Show 'kora install --mode dev --dry-run' $r
Assert-That ($r.Code -eq 0) 'dev dry-run exits 0'
Assert-That ($r.Output -match 'mode:\s+development') 'plan says development mode'
Assert-That ($r.Output -match 'build\.forks') 'plan includes the fork step'
Assert-That ($r.Output -match 'build\.resources') 'plan includes the clone step'
Assert-That (-not (Test-Path 'C:\kora-smoke\dev')) 'dry run created nothing'
