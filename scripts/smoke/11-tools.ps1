# Kora's own git and gh (no administrator rights, no PATH change): with git and gh hidden from PATH, `kora tools install`
# downloads MinGit and the GitHub CLI over HTTPS, verifies the pinned checksums and unpacks them under
# %LOCALAPPDATA%\Kora\tools; then git and gh run from there although PATH never listed them, and `kora uninstall`
# removes them. Uses a throw-away KORA_HOME so nothing real is touched.
. "$PSScriptRoot/common.ps1"
$kora = Get-KoraExe
$home_ = Join-Path $env:RUNNER_TEMP "kora-tools-smoke"
if (-not $env:RUNNER_TEMP) { $home_ = Join-Path ([IO.Path]::GetTempPath()) "kora-tools-smoke" }
Remove-Item -Recurse -Force $home_ -ErrorAction SilentlyContinue
$bare = "$env:SystemRoot\System32;$env:SystemRoot;$env:SystemRoot\System32\WindowsPowerShell\v1.0"
$env_ = @{ KORA_HOME = $home_; PATH = $bare }

$r = Invoke-Native $kora @('tools', 'status') $env_; Show 'kora tools status (git and gh hidden)' $r
Assert-That ($r.Code -eq 1) 'status says something is missing (exit 1)'
Assert-That ($r.Output -match 'Git: not installed') 'Git is reported as not installed'

$r = Invoke-Native $kora @('tools', 'install') $env_; Show 'kora tools install' $r
Assert-That ($r.Code -eq 0) 'install exits 0'
Assert-That (Test-Path (Join-Path $home_ 'tools\git\cmd\git.exe')) 'MinGit is unpacked into the Kora folder'
Assert-That (Test-Path (Join-Path $home_ 'tools\gh\bin\gh.exe')) 'the GitHub CLI is unpacked into the Kora folder'
Assert-That (-not (Get-ChildItem (Join-Path $home_ 'tools') -Force | Where-Object { $_.Name -like '.dl-*' -or $_.Name -like '.stage-*' })) 'no partial downloads are left behind'

$r = Invoke-Native $kora @('tools', 'status') $env_; Show 'kora tools status (after)' $r
Assert-That ($r.Code -eq 0) 'status is fine now'
Assert-That ($r.Output -match "Git: \d+\.\d+.*Kora's own copy") 'git runs from Kora''s own copy without any PATH change'
Assert-That ($r.Output -match "GitHub CLI: \d+\.\d+.*Kora's own copy") 'gh runs from Kora''s own copy without any PATH change'

$r = Invoke-Native $kora @('check') $env_; Show 'kora check' $r
Assert-That ($r.Output -match 'Git: Installed') 'the system check lists Git'

$r = Invoke-Native $kora @('uninstall', '--yes') $env_; Show 'kora uninstall --yes' $r
Assert-That (-not (Test-Path (Join-Path $home_ 'tools'))) 'uninstall removes Kora''s own copies'
Remove-Item -Recurse -Force $home_ -ErrorAction SilentlyContinue
exit 0
