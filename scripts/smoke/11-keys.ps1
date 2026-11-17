# Saved licence keys on a real Windows machine (0.7.0): secrets.json is created with its inherited permissions removed
# (no Users / Everyone / Authenticated Users), the list only shows the last four characters, the same key twice is
# refused, an install can name a saved key, and rename/remove work. Uses a throw-away KORA_HOME.
. "$PSScriptRoot/common.ps1"

$kora = Get-KoraExe
$home1 = Join-Path ([IO.Path]::GetTempPath()) ("kora-keys-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $home1 | Out-Null
$key = 'cfxk_SAVEDSAVEDSAVEDSAVED_1234'
$e = @{ KORA_HOME = $home1; CFX_LICENSE_KEY = $key }

$r = Invoke-Native $kora @('keys', 'add', 'Main') $e; Show 'keys add' $r
Assert-That ($r.Code -eq 0) 'keys add exits 0'
Assert-That ($r.Output -notmatch [regex]::Escape($key)) 'add never prints the key'
$file = Join-Path $home1 'secrets.json'
Assert-That (Test-Path $file) 'secrets.json was written in the Kora folder'
$acl = Get-Acl-Text $file; Write-Host $acl
Assert-That ($acl -notmatch '(?i)\\Users:|Everyone|Authenticated Users|BUILTIN\\Users') 'secrets.json ACL does not grant Users/Everyone/Authenticated Users'
Assert-That ($acl -match [regex]::Escape($env:USERNAME)) 'secrets.json ACL names the signed-in user'
$readOk = $false; try { [void][IO.File]::ReadAllText($file); $readOk = $true } catch {}
Assert-That $readOk 'the signed-in user can still read secrets.json'

$r = Invoke-Native $kora @('keys', 'list') $e; Show 'keys list' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match '1234') 'list shows the last four characters'
Assert-That ($r.Output -notmatch 'SAVEDSAVED') 'list never shows the key'

$r = Invoke-Native $kora @('keys', 'add', 'Again') $e; Show 'keys add (same key)' $r
Assert-That ($r.Code -eq 1 -and $r.Output -match 'already saved as') 'the same key twice is refused, naming the saved one'

$r = Invoke-Native $kora @('install', '--name', 'named', '--dir', (Join-Path $home1 'srv'), '--db', 'skip', '--license-key-name', 'main', '--dry-run', '-y') $e; Show 'install --license-key-name (dry run)' $r
Assert-That ($r.Code -eq 0) 'install with a saved key name (dry run) exits 0'
Assert-That ($r.Output -notmatch 'SAVEDSAVED') 'the plan never prints the key'
$r = Invoke-Native $kora @('install', '--name', 'named', '--dir', (Join-Path $home1 'srv'), '--license-key-name', 'nope', '--dry-run', '-y') $e; Show 'install --license-key-name nope' $r
Assert-That ($r.Code -eq 2 -and $r.Output -match 'no saved licence key called') 'an unknown key name is refused with a plain message'

$r = Invoke-Native $kora @('keys', 'rename', 'main', 'Live') $e; Assert-That ($r.Code -eq 0) 'rename exits 0'
$r = Invoke-Native $kora @('keys', 'remove', 'live', '-y') $e; Assert-That ($r.Code -eq 0) 'remove exits 0'
Assert-That ((Get-Content -Raw $file) -notmatch 'SAVEDSAVED') 'the file no longer holds the key'
Remove-Item -Recurse -Force $home1
