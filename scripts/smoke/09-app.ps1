# The Kora app on a real Windows machine: kora.exe in Program Files, machine PATH, Start Menu + Desktop
# shortcuts (Run as administrator), the Apps & features entry, replace, self-uninstall, full cleanup.
. "$PSScriptRoot/common.ps1"

$kora = Get-KoraExe
$dir = Join-Path $env:ProgramFiles 'Kora'
$exe = Join-Path $dir 'kora.exe'
$startLnk = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Kora.lnk'
$deskLnk = Join-Path $env:PUBLIC 'Desktop\Kora.lnk'
$regPath = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Kora'
$envKey = 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment'

function Get-RawMachinePath { (Get-Item $envKey).GetValue('Path', '', 'DoNotExpandEnvironmentNames') }
function Read-Lnk([string]$Path) {
    $s = (New-Object -ComObject WScript.Shell).CreateShortcut($Path)
    $bytes = [IO.File]::ReadAllBytes($Path)
    [pscustomobject]@{ Target = $s.TargetPath; Args = $s.Arguments; Work = $s.WorkingDirectory; Icon = $s.IconLocation; Admin = (($bytes[0x15] -band 0x20) -ne 0) }
}
function Count-KoraOnPath { @((Get-RawMachinePath) -split ';' | Where-Object { $_.TrimEnd('\') -ieq $dir }).Count }

Assert-That (-not (Test-Path $exe)) 'precondition: no Kora app installed yet'
$pathBefore = Get-RawMachinePath
$kindBefore = (Get-Item $envKey).GetValueKind('Path')

# ---- status + dry run change nothing ----
$r = Invoke-Native $kora @('app', 'status'); Show 'app status (before)' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match 'installed: no') 'status says not installed'
$r = Invoke-Native $kora @('app', 'install', '--yes', '--dry-run'); Show 'app install --dry-run' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match 'Dry run' -and $r.Output -match 'machine PATH' -and $r.Output -match 'Apps & features') 'dry run lists the changes'
Assert-That (-not (Test-Path $exe) -and -not (Test-Path $startLnk) -and -not (Test-Path $regPath)) 'dry run changed nothing'

# ---- install ----
$r = Invoke-Native $kora @('app', 'install', '--yes'); Show 'app install' $r
Assert-That ($r.Code -eq 0) 'app install exits 0'
Assert-That (Test-Path $exe) 'kora.exe is in Program Files\Kora'
Assert-That ((Get-FileHash $exe).Hash -eq (Get-FileHash $kora).Hash) 'the installed copy is the running program'
Assert-That (Test-Path (Join-Path $dir 'app.json')) 'app state file written'
Assert-That ((Count-KoraOnPath) -eq 1) 'machine PATH lists the Kora folder exactly once'
Assert-That ((Get-Item $envKey).GetValueKind('Path') -eq $kindBefore) "machine PATH keeps its registry type ($kindBefore)"
Assert-That ((Get-RawMachinePath).StartsWith($pathBefore.TrimEnd(';'))) 'existing PATH entries are untouched'

foreach ($pair in @(@('Start Menu', $startLnk), @('Desktop', $deskLnk))) {
    $l = Read-Lnk $pair[1]
    Assert-That (Test-Path $pair[1]) "$($pair[0]) shortcut exists"
    Assert-That ($l.Target -ieq $exe) "$($pair[0]) shortcut targets kora.exe ($($l.Target))"
    Assert-That ($l.Args -eq 'manager') "$($pair[0]) shortcut runs 'kora.exe manager'"
    Assert-That ($l.Work -ieq $dir) "$($pair[0]) shortcut working dir is the program folder"
    Assert-That ($l.Icon -like "$exe*") "$($pair[0]) shortcut has the Kora icon ($($l.Icon))"
    Assert-That $l.Admin "$($pair[0]) shortcut has the Run as administrator flag"
}

$k = Get-ItemProperty $regPath
Assert-That ($k.DisplayName -eq 'Kora') 'Apps & features: DisplayName'
Assert-That ($k.Publisher -eq 'Kora') 'Apps & features: Publisher'
Assert-That ($k.DisplayVersion -match '^\d+\.\d+\.\d+') "Apps & features: DisplayVersion ($($k.DisplayVersion))"
Assert-That ($k.InstallLocation -ieq $dir) 'Apps & features: InstallLocation'
Assert-That ($k.UninstallString -eq ('"' + $exe + '" uninstall --app')) "Apps & features: UninstallString ($($k.UninstallString))"
Assert-That ($k.DisplayIcon -like "$exe*") 'Apps & features: DisplayIcon'
Assert-That ($k.NoModify -eq 1 -and $k.NoRepair -eq 1) 'Apps & features: NoModify/NoRepair'

# The exe carries the Kora icon resource (the shortcut and Apps & features icons point at it).
Add-Type -AssemblyName System.Drawing
$icon = [System.Drawing.Icon]::ExtractAssociatedIcon($exe)
Assert-That ($null -ne $icon -and $icon.Width -ge 16) 'kora.exe has an icon'

# ---- a fresh process finds `kora` through the machine PATH ----
$ver = (& $kora --version) -join ''
$out = & "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -Command '$env:Path = [Environment]::GetEnvironmentVariable("Path","Machine"); (Get-Command kora).Source; kora --version' 2>&1
Write-Host ($out -join "`n")
Assert-That (($out -join "`n") -match [regex]::Escape($exe)) 'a fresh process resolves `kora` to Program Files\Kora\kora.exe'
Assert-That (($out -join "`n") -match [regex]::Escape($ver)) "and runs it ($ver)"
Assert-That (((& $exe --version) -join '') -eq $ver) 'the installed copy reports the same version'

$r = Invoke-Native $kora @('app', 'status'); Show 'app status (after)' $r
foreach ($want in 'installed: yes', 'on_path: yes', 'start_menu_shortcut: yes', 'desktop_shortcut: yes', 'apps_and_features: yes') {
    Assert-That ($r.Output -match [regex]::Escape($want)) "status: $want"
}

# ---- idempotent ----
$r = Invoke-Native $kora @('app', 'install', '--yes'); Show 'app install again' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match 'already installed and up to date') 'second install changes nothing'
Assert-That ((Count-KoraOnPath) -eq 1) 'PATH entry not duplicated'

# ---- replace a different build (same version, other bytes) ----
$mod = Join-Path ([IO.Path]::GetTempPath()) 'kora-mod.exe'
Copy-Item $kora $mod -Force
[IO.File]::WriteAllBytes($mod, ([IO.File]::ReadAllBytes($mod) + [byte[]](0, 0, 0, 0)))
$r = Invoke-Native $mod @('app', 'install', '--yes'); Show 'app install (different build)' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match 'Replace') 'a different build replaces the installed copy'
Assert-That ((Get-FileHash $exe).Hash -eq (Get-FileHash $mod).Hash) 'the installed copy is now the new build'
Assert-That (((& $exe --version) -join '') -eq $ver) 'and still runs'
Assert-That (@(Get-ChildItem $dir -Filter 'kora.exe.new' -ErrorAction SilentlyContinue).Count -eq 0) 'no temporary copy left behind'
Remove-Item $mod -Force

# ---- uninstall from the installed copy itself (what Apps & features does) ----
$r = Invoke-Native $exe @('uninstall', '--app', '--yes'); Show 'installed kora.exe uninstall --app' $r
Assert-That ($r.Code -eq 0) 'uninstall --app from the installed copy exits 0'
Assert-That (-not (Test-Path $exe)) 'kora.exe is gone from Program Files\Kora'
Assert-That (-not (Test-Path $startLnk) -and -not (Test-Path $deskLnk)) 'shortcuts removed'
Assert-That (-not (Test-Path $regPath)) 'Apps & features entry removed'
Assert-That ((Count-KoraOnPath) -eq 0) 'machine PATH entry removed'
Assert-That ((Get-RawMachinePath).TrimEnd(';') -ceq $pathBefore.TrimEnd(';')) 'machine PATH is exactly as before the install'
Assert-That ((Get-Item $envKey).GetValueKind('Path') -eq $kindBefore) 'machine PATH registry type unchanged'

# ---- final cleanup through the CLI: nothing is left (also sweeps a renamed, then-deleted self copy) ----
$r = Invoke-Native $kora @('app', 'uninstall', '--yes'); Show 'app uninstall' $r
Assert-That ($r.Code -eq 0) 'app uninstall exits 0'
Assert-That (-not (Test-Path $dir)) 'the Program Files\Kora folder is gone'
$r = Invoke-Native $kora @('app', 'status'); Show 'app status (final)' $r
Assert-That ($r.Output -match 'installed: no' -and $r.Output -match 'on_path: no' -and $r.Output -match 'apps_and_features: no') 'status: nothing installed'
Write-Host 'Kora app smoke test passed.'
