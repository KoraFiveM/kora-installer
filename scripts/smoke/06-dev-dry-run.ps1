. "$PSScriptRoot/common.ps1"
$kora = Get-KoraExe
$env:KORA_CATALOG = (Resolve-Path "$PWD/tests/fixtures/smoke-catalog.json").Path
# NEW spelling: --source / --versions / --fork-owner.
$r = Invoke-Native $kora @('install', '--source', 'fork', '--versions', 'latest', '--fork-owner', 'smoke-owner', '--dir', 'C:\kora-smoke\dev', '--db', 'skip', '--dry-run', '-y') @{ CFX_LICENSE_KEY = 'cfxk_FAKE' }
Show 'kora install --source fork --versions latest --dry-run' $r
Assert-That ($r.Code -eq 0) 'fork + latest dry-run exits 0'
Assert-That ($r.Output -match 'copies:\s+your own copies') 'plan says your own copies'
Assert-That ($r.Output -match 'versions:\s+the latest changes') 'plan says the latest changes'
Assert-That ($r.Output -match 'build\.forks') 'plan includes the fork step'
Assert-That ($r.Output -match 'build\.resources') 'plan includes the clone step'
Assert-That (-not (Test-Path 'C:\kora-smoke\dev')) 'dry run created nothing'

# OLD spelling on purpose: --mode dev and --github-owner must keep working and mean the same.
$r = Invoke-Native $kora @('install', '--mode', 'dev', '--github-owner', 'smoke-owner', '--dir', 'C:\kora-smoke\dev', '--db', 'skip', '--dry-run', '-y') @{ CFX_LICENSE_KEY = 'cfxk_FAKE' }
Show 'kora install --mode dev --dry-run (old spelling)' $r
Assert-That ($r.Code -eq 0) 'old --mode dev dry-run exits 0'
Assert-That ($r.Output -match 'copies:\s+your own copies') 'the old flag still means your own copies'
Assert-That ($r.Output -match 'versions:\s+the latest changes') 'the old flag still means the latest changes'
Assert-That ($r.Output -match 'build\.forks') 'plan includes the fork step'
Assert-That (-not (Test-Path 'C:\kora-smoke\dev')) 'dry run created nothing'

# Plain clones of released versions need no GitHub at all.
$r = Invoke-Native $kora @('install', '--source', 'clone', '--versions', 'released', '--dir', 'C:\kora-smoke\clone', '--db', 'skip', '--dry-run', '-y') @{ CFX_LICENSE_KEY = 'cfxk_FAKE' }
Show 'kora install --source clone --versions released --dry-run' $r
Assert-That ($r.Code -eq 0) 'clone + released dry-run exits 0'
Assert-That ($r.Output -notmatch 'build\.forks') 'no fork step for plain clones'

exit 0
