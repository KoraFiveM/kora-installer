# Saved licence keys on a real Windows machine: they live in Kora's data file (<Kora folder>\store\kora.db). The store FOLDER is
# created with its inherited permissions removed (no Users / Everyone / Authenticated Users), so the database and the -wal/-shm files
# SQLite makes in it inherit that; the list only shows the last four characters, the same key twice is refused, an install can name
# a saved key, rename/remove work, and an old-format secrets.json (Kora 0.7) is imported once and moved to store\before-sqlite.
# Uses a throw-away KORA_HOME.
. "$PSScriptRoot/common.ps1"

$kora = Get-KoraExe
$home1 = Join-Path ([IO.Path]::GetTempPath()) ("kora-keys-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $home1 | Out-Null
$key = 'cfxk_SAVEDSAVEDSAVEDSAVED_1234'
$e = @{ KORA_HOME = $home1; CFX_LICENSE_KEY = $key }

$r = Invoke-Native $kora @('keys', 'add', 'Main') $e; Show 'keys add' $r
Assert-That ($r.Code -eq 0) 'keys add exits 0'
Assert-That ($r.Output -notmatch [regex]::Escape($key)) 'add never prints the key'
$dir = Join-Path $home1 'store'
$file = Join-Path $dir 'kora.db'
Assert-That (Test-Path $file) 'the data file was created in the Kora folder (store\kora.db)'
Assert-That (-not (Test-Path (Join-Path $home1 'secrets.json'))) 'no secrets.json is written any more'
$acl = Get-Acl-Text $dir; Write-Host $acl
Assert-That ($acl -notmatch '(?i)\\Users:|Everyone|Authenticated Users|BUILTIN\\Users') 'the store folder ACL does not grant Users/Everyone/Authenticated Users'
Assert-That ($acl -match [regex]::Escape($env:USERNAME)) 'the store folder ACL names the signed-in user'
$aclFile = Get-Acl-Text $file; Write-Host $aclFile
Assert-That ($aclFile -notmatch '(?i)\\Users:|Everyone|Authenticated Users|BUILTIN\\Users') 'the data file ACL does not grant Users/Everyone/Authenticated Users either'
$readOk = $false; try { [void][IO.File]::ReadAllBytes($file); $readOk = $true } catch {}
Assert-That $readOk 'the signed-in user can still read the data file'

$r = Invoke-Native $kora @('keys', 'list') $e; Show 'keys list' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match '1234') 'list shows the last four characters'
Assert-That ($r.Output -notmatch 'SAVEDSAVED') 'list never shows the key'

# DPAPI (Windows' protected storage for this account): the key is sealed in the data file, `keys keychain` says where it is kept, and
# turning it off moves the value back in clear (and on seals it again). Written on Linux, unverified there: this is the real check.
$r = Invoke-Native $kora @('keys', 'keychain', 'status') $e; Show 'keys keychain status' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match 'Windows') 'the status says the key is kept in Windows protected storage'
$sealed = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($file))
Assert-That ($sealed -notmatch 'SAVEDSAVED') 'the data file holds the key sealed (DPAPI), not in clear'
$r = Invoke-Native $kora @('keys', 'list') $e; Assert-That ($r.Output -match 'Windows') 'keys list says where the key is kept'
$r = Invoke-Native $kora @('keys', 'keychain', 'off') $e; Show 'keys keychain off' $r
Assert-That ($r.Code -eq 0) 'keychain off exits 0'
$r = Invoke-Native $kora @('keys', 'keychain', 'on') $e; Show 'keys keychain on' $r
Assert-That ($r.Code -eq 0) 'keychain on exits 0'
$r = Invoke-Native $kora @('install', '--name', 'named2', '--dir', (Join-Path $home1 'srv2'), '--db', 'skip', '--license-key-name', 'main', '--dry-run', '-y') $e; Show 'install with a sealed key (dry run)' $r
Assert-That ($r.Code -eq 0) 'a sealed saved key is still found by name'

$r = Invoke-Native $kora @('keys', 'add', 'Again') $e; Show 'keys add (same key)' $r
Assert-That ($r.Code -eq 1 -and $r.Output -match 'already saved as') 'the same key twice is refused, naming the saved one'

$r = Invoke-Native $kora @('install', '--name', 'named', '--dir', (Join-Path $home1 'srv'), '--db', 'skip', '--license-key-name', 'main', '--dry-run', '-y') $e; Show 'install --license-key-name (dry run)' $r
Assert-That ($r.Code -eq 0) 'install with a saved key name (dry run) exits 0'
Assert-That ($r.Output -notmatch 'SAVEDSAVED') 'the plan never prints the key'
$r = Invoke-Native $kora @('install', '--name', 'named', '--dir', (Join-Path $home1 'srv'), '--license-key-name', 'nope', '--dry-run', '-y') $e; Show 'install --license-key-name nope' $r
Assert-That ($r.Code -eq 2 -and $r.Output -match 'no saved licence key called') 'an unknown key name is refused with a plain message'

$r = Invoke-Native $kora @('keys', 'rename', 'main', 'Live') $e; Assert-That ($r.Code -eq 0) 'rename exits 0'
$r = Invoke-Native $kora @('keys', 'remove', 'live', '-y') $e; Assert-That ($r.Code -eq 0) 'remove exits 0'
$bytes = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($file))
Assert-That ($bytes -notmatch 'SAVEDSAVED') 'the data file no longer holds the key'
$r = Invoke-Native $kora @('audit') $e; Show 'audit' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match 'key_add' -and $r.Output -notmatch 'SAVEDSAVED') 'the audit log recorded the key being added and never holds a key'

# --- an old-format secrets.json (Kora 0.7) is imported once and moved aside, privately ----------------------------
$home2 = Join-Path ([IO.Path]::GetTempPath()) ("kora-keys-old-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $home2 | Out-Null
$old = '{"version":1,"keys":[{"name":"Old main","key":"cfxk_OLDOLDOLDOLDOLDOLD_5678","added":1700000001}]}'
Set-Content -Path (Join-Path $home2 'secrets.json') -Value $old -NoNewline
$e2 = @{ KORA_HOME = $home2 }
$r = Invoke-Native $kora @('keys', 'list') $e2; Show 'keys list (after the upgrade)' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match 'Old main' -and $r.Output -match '5678') 'the old key is listed after the upgrade'
Assert-That ($r.Output -notmatch 'OLDOLDOLD') 'and never shown in full'
Assert-That (-not (Test-Path (Join-Path $home2 'secrets.json'))) 'the old secrets.json was moved out of the Kora folder'
$kept = Join-Path $home2 'store\before-sqlite\secrets.json'
Assert-That (Test-Path $kept) 'it is kept (not deleted) in store\before-sqlite'
$aclKept = Get-Acl-Text $kept; Write-Host $aclKept
Assert-That ($aclKept -notmatch '(?i)\\Users:|Everyone|Authenticated Users|BUILTIN\\Users') 'the kept copy is private too'
Remove-Item -Recurse -Force $home1, $home2
