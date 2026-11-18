<#
.SYNOPSIS
  Checks a built Kora MSI WITHOUT installing it (build-msi.ps1 runs this after every build; CI therefore does too):

    * Windows Installer's own validation (ICE rules) through `wix msi validate`. Three rules are switched off, for a written reason:
      ICE38, ICE43 and ICE57 assume that a shortcut in the Start Menu or on the Desktop is per-user data, because the
      folders sit in the user's profile. This package is per-machine (ALLUSERS=1 is in the Property table): those folders are
      the All Users ones, so the shortcuts' registry key path correctly is under HKLM, which is what WiX documents for a
      per-machine shortcut. Every other rule must pass.
    * the component GUIDs, which must never change (an upgrade identifies a component by its GUID: a different one makes
      the next version remove the old component and add a new one, which stops and removes the service in between);
    * the upgrade code, that a major upgrade is possible and that a downgrade is refused;
    * how the service is registered (own process, LocalSystem, starts with Windows, `service serve`, started on install,
      stopped on upgrade and uninstall, removed on uninstall);
    * the order of the custom actions: the failure flag between InstallServices and StartServices; allowing the installing
      account and the migration after the files and the service are in place, never when removing; the desktop app's
      start-up entry is not taken out by an upgrade.

  packaging/windows/check-msi.ps1 -Msi dist\kora-windows-x86_64.msi [-WithDesktop]
#>
param(
    [Parameter(Mandatory)][string]$Msi,
    # The MSI was built with the desktop app (-DesktopExe).
    [switch]$WithDesktop
)
$ErrorActionPreference = 'Stop'
$Msi = (Resolve-Path $Msi).Path

# ---- 1. validation ------------------------------------------------------------------------------------------------
$out = & wix msi validate -sice ICE38 -sice ICE43 -sice ICE57 $Msi 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { throw "wix msi validate found problems in ${Msi}:`n$out" }
Write-Host 'ICE validation: no errors (ICE38, ICE43, ICE57 are off for the reason given in check-msi.ps1)'

# ---- 2. the tables ------------------------------------------------------------------------------------------------
$installer = New-Object -ComObject WindowsInstaller.Installer
$db = $installer.OpenDatabase($Msi, 0)
# One column per query (what the COM API returns reliably from PowerShell).
function Get-Column([string]$sql) {
    $view = $db.OpenView($sql)
    $null = $view.Execute()
    $rows = @()
    while ($true) {
        $rec = $view.Fetch()
        if ($null -eq $rec) { break }
        $rows += $rec.StringData(1)
    }
    $null = $view.Close()
    , $rows
}
function Get-Value([string]$sql) { $r = Get-Column $sql; if ($r.Count -eq 0) { $null } else { $r[0] } }

$problems = @()
function Expect($what, $got, $want) { if ("$got" -ne "$want") { $script:problems += "$what is '$got', expected '$want'" } }

# Component GUIDs: never change these (see above). A NEW component gets a new line here.
$guids = [ordered]@{
    Program           = '{1513A06D-2D5D-5626-8ADA-ABEF986A9E1E}'
    PathEntry         = '{9D0F3B1A-7C52-4E6B-8A14-5F2C0D7E9B63}'
    DataFolder        = '{3A7E5C91-2B64-4D08-9E1F-6C8A4B0D2F75}'
    StartMenuShortcut = '{C41B8E2D-5F90-4A37-B6D1-0E9A7C3F8A52}'
    DesktopShortcut   = '{E7D2A049-1B38-4C65-8F90-3D5B6A1C9E04}'
}
if ($WithDesktop) {
    $guids['DesktopApp'] = '{DESKTOPAPP-GUID}'
    $guids['DesktopAppStartMenuShortcut'] = '{4B7A1D52-8C3E-4F69-A0D7-5E2B9C6F1A83}'
    $guids['DesktopAppDesktopShortcut'] = '{9E3C5A17-2D84-4B06-8F1A-7C0D6B4E2A95}'
}
$components = Get-Column 'SELECT Component FROM Component'
foreach ($name in $guids.Keys) {
    Expect "component $name GUID" (Get-Value "SELECT ComponentId FROM Component WHERE Component='$name'") $guids[$name]
}
foreach ($c in $components) { if (-not $guids.Contains($c)) { $problems += "component $c is not in the list of frozen GUIDs in check-msi.ps1" } }

# Upgrade code, per-machine, the major upgrade and the downgrade refusal.
Expect 'UpgradeCode' (Get-Value "SELECT Value FROM Property WHERE Property='UpgradeCode'") '{6B2F7C5A-3E1D-4B8A-9C4F-2A7D5E8B1C30}'
Expect 'ALLUSERS' (Get-Value "SELECT Value FROM Property WHERE Property='ALLUSERS'") '1'
Expect 'rows in the Upgrade table (older versions, newer versions)' (Get-Column 'SELECT UpgradeCode FROM Upgrade').Count 2
Expect 'downgrade launch condition' (Get-Value 'SELECT Condition FROM LaunchCondition') 'NOT WIX_DOWNGRADE_DETECTED'

# The service.
Expect 'service name' (Get-Value 'SELECT Name FROM ServiceInstall') 'KoraService'
Expect 'service type (own process)' (Get-Value 'SELECT ServiceType FROM ServiceInstall') '16'
Expect 'service start type (automatic)' (Get-Value 'SELECT StartType FROM ServiceInstall') '2'
Expect 'service account' (Get-Value 'SELECT StartName FROM ServiceInstall') 'LocalSystem'
Expect 'service arguments' (Get-Value 'SELECT Arguments FROM ServiceInstall') 'service serve'
Expect 'service component' (Get-Value 'SELECT Component_ FROM ServiceInstall') 'Program'
# ServiceControl.Event: 0x1 start on install, 0x2 stop on install, 0x10 stop on uninstall, 0x20 delete on uninstall
Expect 'service control (start+stop on install, stop+delete on uninstall; waits)' ((Get-Value 'SELECT Event FROM ServiceControl') + '/' + (Get-Value 'SELECT Wait FROM ServiceControl')) '51/1'

# The order of the actions.
$seq = Get-Column 'SELECT Action FROM InstallExecuteSequence ORDER BY Sequence'
function Pos($a) { $i = [array]::IndexOf($seq, $a); if ($i -lt 0) { $script:problems += "action $a is not in the install sequence" }; $i }
if ((Pos 'InstallServices') -ge (Pos 'KoraFailureFlag') -or (Pos 'KoraFailureFlag') -ge (Pos 'StartServices')) { $problems += 'KoraFailureFlag must run after InstallServices and before StartServices' }
if ((Pos 'StartServices') -ge (Pos 'KoraEnrolUsers') -or (Pos 'InstallFiles') -ge (Pos 'KoraEnrolUsers')) { $problems += 'KoraEnrolUsers must run after the files and the service are in place' }
if ((Pos 'KoraEnrolUsers') -ge (Pos 'InstallFinalize') -or (Pos 'KoraMigrate') -ge (Pos 'InstallFinalize')) { $problems += 'KoraEnrolUsers and KoraMigrate must run before InstallFinalize' }
if ((Pos 'InstallExecute') -ge (Pos 'RemoveExistingProducts')) { $problems += 'RemoveExistingProducts must run after InstallExecute (the new files are in before the old product goes: the data folder and the service stay on an upgrade)' }
# CustomAction.Type: 18 = exe from an installed file, 0x40 ignore the result, 0x400 deferred, 0x800 not impersonated.
foreach ($a in 'KoraFailureFlag', 'KoraEnrolUsers', 'KoraMigrate') {
    $cond = Get-Value "SELECT Condition FROM InstallExecuteSequence WHERE Action='$a'"
    if ($cond -notmatch 'NOT REMOVE') { $problems += "$a must not run when Kora is being removed (condition is '$cond')" }
    $type = [int](Get-Value "SELECT Type FROM CustomAction WHERE Action='$a'")
    $want = if ($a -eq 'KoraFailureFlag') { 34 } else { 18 }   # 34 = exe with a working directory
    Expect "$a type (deferred, as the system, result ignored)" ($type -band 0xF7F) ($want -bor 0x40 -bor 0x400 -bor 0x800)
}
if ($WithDesktop) {
    $cond = Get-Value "SELECT Condition FROM InstallExecuteSequence WHERE Action='KoraDesktopUninstall'"
    if ($cond -notmatch 'UPGRADINGPRODUCTCODE') { $problems += "KoraDesktopUninstall would also run when an upgrade removes the old version (condition is '$cond')" }
}
if ($problems.Count -gt 0) { throw ("The MSI is not what it must be:`n - " + ($problems -join "`n - ")) }
Write-Host "MSI checks passed: $($guids.Count) component GUIDs, upgrade, service, action order"
