# The Kora shortcuts on a real Windows machine (0.7.0): per-user Start Menu + Desktop shortcuts that run a
# bare `kora` (no "Run as administrator", no machine-wide copy), and tidying up the OLD machine-wide app
# (kora.exe in Program Files, machine PATH, admin shortcuts, Apps & features entry) that 0.6 and earlier made.
. "$PSScriptRoot/common.ps1"

$kora = Get-KoraExe
$startLnk = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Kora.lnk'
$desktopDir = [Environment]::GetFolderPath('Desktop')
$deskLnk = Join-Path $desktopDir 'Kora.lnk'

function Read-Lnk([string]$Path) {
    $s = (New-Object -ComObject WScript.Shell).CreateShortcut($Path)
    $bytes = [IO.File]::ReadAllBytes($Path)
    [pscustomobject]@{ Target = $s.TargetPath; Args = $s.Arguments; Work = $s.WorkingDirectory; Icon = $s.IconLocation; Style = $s.WindowStyle; Admin = (($bytes[0x15] -band 0x20) -ne 0) }
}

Assert-That (-not (Test-Path $startLnk)) 'precondition: no per-user Kora shortcut yet'

# ---- status + dry run change nothing ----
$r = Invoke-Native $kora @('app', 'status'); Show 'app status (before)' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match 'shortcut_start_menu: no') 'status says there is no shortcut yet'
$r = Invoke-Native $kora @('app', 'install', '--yes', '--dry-run'); Show 'app install --dry-run' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match 'Dry run' -and $r.Output -match 'shortcut') 'dry run lists the changes'
Assert-That (-not (Test-Path $startLnk)) 'dry run changed nothing'

# ---- per-user shortcuts: no administrator rights needed ----
$r = Invoke-Native $kora @('app', 'install', '--yes'); Show 'app install' $r
Assert-That ($r.Code -eq 0) 'app install exits 0'
foreach ($pair in @(@('Start Menu', $startLnk), @('Desktop', $deskLnk))) {
    Assert-That (Test-Path $pair[1]) "$($pair[0]) shortcut exists"
    $l = Read-Lnk $pair[1]
    Assert-That ($l.Target -ieq $kora) "$($pair[0]) shortcut targets kora.exe ($($l.Target))"
    Assert-That ($l.Args -eq '') "$($pair[0]) shortcut runs a bare kora (the panel), not 'manager'"
    Assert-That (-not $l.Admin) "$($pair[0]) shortcut does NOT run as administrator"
    Assert-That ($l.Style -eq 7) "$($pair[0]) shortcut starts minimised (no console window flashing)"
    Assert-That ($l.Icon -like "$kora*") "$($pair[0]) shortcut has the Kora icon"
}
Assert-That (-not (Test-Path (Join-Path $env:ProgramFiles 'Kora\kora.exe'))) 'nothing was put in Program Files'
$r = Invoke-Native $kora @('app', 'status'); Show 'app status (after)' $r
Assert-That ($r.Output -match 'shortcut_start_menu: yes' -and $r.Output -match 'old_app_present: no') 'status: shortcut in place, no old app'
$r = Invoke-Native $kora @('app', 'install', '--yes'); Show 'app install again' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match 'already in place') 'second install changes nothing'

# ---- the old machine-wide app is found, and removed by `uninstall --app` ----
$dir = Join-Path $env:ProgramFiles 'Kora'
$old = Join-Path $dir 'kora.exe'
$oldStart = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Kora.lnk'
$regPath = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Kora'
New-Item -ItemType Directory -Path $dir -Force | Out-Null
Copy-Item $kora $old -Force
$s = (New-Object -ComObject WScript.Shell).CreateShortcut($oldStart)
$s.TargetPath = $old; $s.Arguments = 'manager'; $s.WorkingDirectory = $dir; $s.Save()
$bytes = [IO.File]::ReadAllBytes($oldStart); $bytes[0x15] = $bytes[0x15] -bor 0x20; [IO.File]::WriteAllBytes($oldStart, $bytes)
New-Item -Path $regPath -Force | Out-Null
Set-ItemProperty -LiteralPath $regPath -Name DisplayName -Value 'Kora'
Set-ItemProperty -LiteralPath $regPath -Name Publisher -Value 'Kora'
$r = Invoke-Native $kora @('app', 'status'); Show 'app status (old app present)' $r
Assert-That ($r.Output -match 'old_app_present: yes') 'the old app is detected'
$r = Invoke-Native $kora @('uninstall', '--app', '--yes'); Show 'uninstall --app' $r
Assert-That ($r.Code -eq 0) 'uninstall --app exits 0'
Assert-That (-not (Test-Path $old) -and -not (Test-Path $oldStart) -and -not (Test-Path $regPath)) 'the old copy, its shortcut and its Apps & features entry are gone'
Assert-That (Test-Path $startLnk) 'the per-user shortcut is still there'
$r = Invoke-Native $kora @('uninstall', '--app', '--yes'); Show 'uninstall --app (nothing left)' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match 'nothing to remove') 'with nothing old left it is a no-op'

# ---- `app uninstall` removes Kora's own per-user shortcuts (and leaves the program) ----
$r = Invoke-Native $kora @('app', 'uninstall', '--dry-run'); Show 'app uninstall --dry-run' $r
Assert-That ($r.Code -eq 0 -and (Test-Path $startLnk)) 'a dry run removes nothing'
$r = Invoke-Native $kora @('app', 'uninstall', '--yes'); Show 'app uninstall' $r
Assert-That ($r.Code -eq 0) 'app uninstall exits 0'
Assert-That (-not (Test-Path $startLnk) -and -not (Test-Path $deskLnk)) 'the Start Menu and Desktop shortcuts are gone'
Assert-That (Test-Path $kora) 'the program itself is still there'
$r = Invoke-Native $kora @('app', 'uninstall', '--yes'); Show 'app uninstall (nothing left)' $r
Assert-That ($r.Code -eq 0 -and $r.Output -match 'nothing to remove') 'with no shortcuts left it is a no-op'

Remove-Item $startLnk, $deskLnk -Force -ErrorAction SilentlyContinue
Write-Host 'Kora shortcuts smoke test passed.'
