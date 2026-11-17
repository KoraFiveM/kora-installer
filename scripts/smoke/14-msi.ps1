<#
  Kora's MSI and administrator service on a real Windows machine (windows-latest runs as an ELEVATED built-in
  administrator, so the unelevated, other-account parts are done through the Task Scheduler as standard test
  accounts through scripts/smoke/lib-accounts.ps1 (secondary logon: the way to get a non-elevated process of another
  account from a non-interactive elevated runner).

  What is checked, in order: install the MSI for real, then the layout (Program Files, service, PATH, shortcuts,
  settings), the service used from STANDARD accounts (one allowed, one not), ownership between two allowed
  accounts, forbidden requests, upgrade keeping the settings and records, and uninstall leaving nothing behind.
  Every step prints what it saw; a failure prints the MSI log, the service state, the system event log and the
  service's own files.

  Usage: 14-msi.ps1 -Msi <new.msi> -OldMsi <older.msi>
#>
param(
    [Parameter(Mandatory)][string]$Msi,
    [Parameter(Mandatory)][string]$OldMsi
)
. "$PSScriptRoot/lib-accounts.ps1"
$Msi = (Resolve-Path $Msi).Path
$OldMsi = (Resolve-Path $OldMsi).Path
$client = (Resolve-Path "$PSScriptRoot/svc-client.ps1").Path
$work = 'C:\kora-smoke-msi'
$pf = [Environment]::GetFolderPath('ProgramFiles') + '\Kora'
$koraExe = "$pf\kora.exe"
$A = 'korasmokea'; $B = 'korasmokeb'; $C = 'korasmokec'

function Show-Diagnostics {
    Write-Host '----- diagnostics -----'
    foreach ($l in 'install.log', 'upgrade.log', 'uninstall.log') { if (Test-Path "$work\$l") { Write-Host "== $l (last 60 lines, errors first)"; Select-String -Path "$work\$l" -Pattern 'Error|Return value 3|failed' | Select-Object -First 15 | ForEach-Object { Write-Host $_.Line }; Get-Content "$work\$l" -Tail 60 } }
    & sc.exe qc KoraService 2>&1 | Out-String | Write-Host
    & sc.exe query KoraService 2>&1 | Out-String | Write-Host
    Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Service Control Manager' } -MaxEvents 15 -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "$($_.TimeCreated) $($_.Message)" }
    foreach ($f in "$pf\data\service.json", "$pf\owners.json", "$pf\data\audit.log") { if (Test-Path $f) { Write-Host "== $f"; Get-Content $f -Tail 40 } else { Write-Host "(no $f)" } }
    if (Test-Path $pf) { Get-ChildItem $pf -Recurse | Select-Object -First 30 FullName | Out-String | Write-Host; (& icacls $pf | Out-String) | Write-Host }
}

# Run a PowerShell script as USER: a standard account, NOT elevated (lib-accounts.ps1).
function Invoke-AsUser([string]$User, [string]$Script, [int]$TimeoutSeconds = 120) {
    Invoke-AsAccount -Kind Account -Script $Script -Dir $work -Credential $accounts[$User].Credential -TimeoutSec $TimeoutSeconds
}
function Svc([string]$User, [string]$Verb, [string[]]$Arguments) {
    $quoted = ($Arguments | ForEach-Object { "'" + ($_ -replace "'", "''") + "'" }) -join ','
    $r = Invoke-AsUser $User "& '$client' -Verb '$Verb' -Arguments $(if ($quoted) { $quoted } else { '@()' }); exit `$LASTEXITCODE"
    Write-Host "--- as $User`: $Verb $($Arguments -join ' ') (exit $($r.Code)) ---"; Write-Host $r.Output
    $r
}

try {
    Write-Host '=== accounts ==='
    $null = Initialize-UacWorkDir -Path $work   # a folder every test account can write
    $accounts = @{}
    foreach ($u in $A, $B, $C) { $accounts[$u] = Initialize-TestAccount -Name $u }
    $sidA = $accounts[$A].Sid; $sidB = $accounts[$B].Sid; $sidC = $accounts[$C].Sid
    Write-Host "A=$sidA B=$sidB C=$sidC"
    $os = Invoke-AsUser $A '[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator); whoami /groups | Select-String "Mandatory Label"'
    Write-Host $os.Output
    Assert-That ($os.Output -match 'False') "the test account runs NOT elevated and not as an administrator"

    Write-Host '=== install the (older) MSI for real: one installation, no prompts ==='
    $p = Start-Process msiexec.exe -ArgumentList "/i `"$OldMsi`" /qn /l*v `"$work\install.log`" ALLOWUSERS=`"$sidA $sidC`"" -Wait -PassThru
    Assert-That ($p.ExitCode -eq 0 -or $p.ExitCode -eq 3010) "msiexec /i succeeded (exit $($p.ExitCode))"

    Write-Host '=== layout ==='
    Assert-That (Test-Path $koraExe) 'kora.exe is in Program Files\Kora'
    $svc = Get-Service KoraService -ErrorAction Stop
    Assert-That ($svc.Status -eq 'Running') "the service is running ($($svc.Status))"
    $qc = (& sc.exe qc KoraService | Out-String); Write-Host $qc
    Assert-That ($qc -match 'AUTO_START') 'it starts with Windows'
    Assert-That ($qc -match 'LocalSystem') 'it runs as LocalSystem'
    Assert-That ($qc -match [regex]::Escape("$koraExe`" service serve") -or $qc -match 'service serve') 'its command runs the program in Program Files with `service serve`'
    $acl = (& icacls $pf | Out-String); Write-Host $acl
    Assert-That ($acl -notmatch 'Everyone:.*\(F\)|Users:.*\((M|F)\)') 'ordinary users cannot change Program Files\Kora'
    Assert-That (([Environment]::GetEnvironmentVariable('Path', 'Machine') -split ';') -contains $pf) 'Kora is on the machine PATH'
    Assert-That (Test-Path "$([Environment]::GetFolderPath('CommonPrograms'))\Kora.lnk") 'the Start Menu shortcut exists'
    $policy = Get-Content "$pf\data\service.json" -Raw
    Assert-That ($policy -match [regex]::Escape($sidA) -and $policy -match [regex]::Escape($sidC)) 'A and C are on the list'
    Assert-That ($policy -notmatch [regex]::Escape($sidB)) 'B is not'
    Assert-That (((Get-Item "$pf\data\service.json").GetAccessControl().Owner) -match 'SYSTEM|Administrators') 'the settings file belongs to SYSTEM or Administrators'
    $pipeSd = (Get-Item '\\.\pipe\kora-service').GetAccessControl()
    Assert-That ($pipeSd.Owner -match 'SYSTEM') "the pipe is owned by SYSTEM ($($pipeSd.Owner))"
    Assert-That ($pipeSd.Sddl -match [regex]::Escape($sidA) -and $pipeSd.Sddl -notmatch [regex]::Escape($sidB)) 'the pipe grants A and not B'
    Write-Host "pipe SDDL: $($pipeSd.Sddl)"

    Write-Host '=== status as standard accounts (nothing is elevated) ==='
    $r = Invoke-AsUser $A "& '$koraExe' service status"; Write-Host $r.Output
    Assert-That ($r.Output -match 'serves this account: yes') 'A: the service serves this account'
    $r = Invoke-AsUser $B "& '$koraExe' service status"; Write-Host $r.Output
    Assert-That ($r.Output -match 'serves this account: no') 'B: running, but not for this account'

    Write-Host '=== A makes a server folder and asks the service for its firewall step ==='
    $srvA = "$work\a\smoke-a"
    $r = Invoke-AsUser $A "New-Item -ItemType Directory -Force -Path '$srvA\.kora' | Out-Null; Set-Content -Path '$srvA\.kora\request.json' -Value '{""mode"":""production"",""source"":""clone"",""versions"":""released"",""name"":""smoke-a"",""dir"":""$($srvA -replace '\\','\\')"",""hostname"":"""",""ports"":{""game"":31120,""txadmin"":41120},""resources"":""All"",""db"":""Skip"",""service"":false,""firewall"":true,""dry_run"":false}'; exit 0"
    $r = Svc $A 'admin-step' @('run', '--step', 'system.firewall', '--dir', $srvA)
    Assert-That ($r.Code -eq 0 -and $r.Output -match 'RESULT ok=true') 'the firewall step ran for A without any prompt'
    $rules = (& netsh advfirewall firewall show rule name=all | Out-String)
    Assert-That ($rules -match 'kora') 'a Kora firewall rule exists'
    $ledger = Get-Content "$pf\owners.json" -Raw
    Assert-That ($ledger -match 'firewall:smoke-a' -and $ledger -match [regex]::Escape($sidA)) 'the ledger says A owns the firewall rules of smoke-a'
    $audit = Get-Content "$pf\data\audit.log" -Raw
    Assert-That ($audit -match 'run system\.firewall \| done') 'the audit log has the operation and its result'
    Assert-That ($audit -notmatch [regex]::Escape($work)) 'the audit log holds no paths'

    Write-Host '=== refusals ==='
    $r = Svc $B 'admin-step' @('run', '--step', 'system.firewall', '--dir', $srvA)
    Assert-That ($r.Code -eq 3 -or $r.Code -eq 5) "B is refused or cannot even open the pipe (exit $($r.Code))"
    $r = Svc $C 'admin-step' @('undo', '--step', 'system.firewall', '--dir', $srvA, '--name', 'smoke-a', '--game-port', '31120', '--txadmin-port', '41120')
    Assert-That ($r.Code -eq 3 -and $r.Output -match 'does not belong to you') 'C (allowed) cannot touch A''s server'
    foreach ($case in @(@('install', @('--dir', $srvA)), @('uninstall', @('--yes')), @('schedule', @('set', 'smoke-a', '--as-system')), @('admin-step', @('run', '--step', 'system.firewall', '--dir', "$work\a\..\a\smoke-a")))) {
        $r = Svc $A $case[0] $case[1]
        Assert-That ($r.Code -eq 3) "refused: $($case[0]) $($case[1] -join ' ')"
    }
    $r = Invoke-AsUser $A "(Get-Content '$srvA\.kora\request.json') -replace '31120','3389' | Set-Content '$srvA\.kora\request.json'; exit 0"
    $r = Svc $A 'admin-step' @('run', '--step', 'system.firewall', '--dir', $srvA)
    Assert-That ($r.Code -eq 3 -and $r.Output -match '3389') 'remote desktop port from a forged record is refused'
    # a junction in the path
    $r = Invoke-AsUser $A "cmd /c mklink /J '$work\a\junction' '$srvA' | Out-Null; exit 0"
    $r = Svc $A 'admin-step' @('run', '--step', 'system.firewall', '--dir', "$work\a\junction")
    Assert-That ($r.Code -eq 3) 'a junction as the folder is refused'
    # the pipe cannot be squatted or doubled by an allowed account
    $r = Invoke-AsUser $A '$s = New-Object System.IO.Pipes.NamedPipeServerStream("kora-service", [System.IO.Pipes.PipeDirection]::InOut, 10); "created"; exit 0'
    Assert-That ($r.Output -notmatch 'created') 'an allowed account cannot make another instance of the pipe'

    Write-Host '=== allow and deny take effect at once ==='
    & $koraExe service deny $C | Out-Host
    $r = Svc $C 'admin-step' @('run', '--step', 'system.firewall', '--dir', "$work\c\none")
    Assert-That ($r.Code -eq 3 -or $r.Code -eq 5) 'C is refused after deny'
    & $koraExe service allow $C | Out-Host
    Assert-That ((& $koraExe service list | Out-String) -match [regex]::Escape($sidC)) 'allow puts C back'

    Write-Host '=== upgrade keeps the settings and the records ==='
    $p = Start-Process msiexec.exe -ArgumentList "/i `"$Msi`" /qn /l*v `"$work\upgrade.log`"" -Wait -PassThru
    Assert-That ($p.ExitCode -eq 0 -or $p.ExitCode -eq 3010) "the upgrade succeeded (exit $($p.ExitCode))"
    Assert-That ((Get-Service KoraService).Status -eq 'Running') 'the service runs again after the upgrade'
    Assert-That ((Get-Content "$pf\data\service.json" -Raw) -match [regex]::Escape($sidA)) 'the list of accounts survived'
    Assert-That ((Get-Content "$pf\owners.json" -Raw) -match 'firewall:smoke-a') 'the ownership records survived'
    $r = Invoke-AsUser $A "& '$koraExe' service status"
    Assert-That ($r.Output -match 'serves this account: yes') 'A still has the service'
    $r = Svc $A 'admin-step' @('undo', '--step', 'system.firewall', '--dir', $srvA, '--name', 'smoke-a', '--game-port', '31120', '--txadmin-port', '41120')
    Assert-That ($r.Code -eq 0) 'A removes their own firewall rules through the service'
    Assert-That ((Get-Content "$pf\owners.json" -Raw) -notmatch 'firewall:smoke-a') 'and the record is gone'

    Write-Host '=== the desktop app (a feature of the newer MSI; the older one has none) ==='
    Assert-That (Test-Path "$pf\kora-desktop.exe") 'kora-desktop.exe is in Program Files\Kora after the upgrade'
    $lnk = (New-Object -ComObject WScript.Shell).CreateShortcut("$([Environment]::GetFolderPath('CommonPrograms'))\Kora.lnk")
    Assert-That ($lnk.TargetPath -eq "$pf\kora-desktop.exe") "the Start Menu shortcut starts the desktop app ($($lnk.TargetPath))"
    Assert-That (-not (Get-ItemProperty 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Run' -Name Kora -ErrorAction SilentlyContinue)) 'the MSI does not write an autostart entry (the app does, per user)'

    Write-Host '=== uninstall leaves nothing behind ==='
    $p = Start-Process msiexec.exe -ArgumentList "/x `"$Msi`" /qn /l*v `"$work\uninstall.log`"" -Wait -PassThru
    Assert-That ($p.ExitCode -eq 0 -or $p.ExitCode -eq 3010) "msiexec /x succeeded (exit $($p.ExitCode))"
    Assert-That (-not (Get-Service KoraService -ErrorAction SilentlyContinue)) 'the service is gone'
    Assert-That (-not (Test-Path $pf)) 'Program Files\Kora is gone (program, settings, records, audit log)'
    Assert-That (-not (([Environment]::GetEnvironmentVariable('Path', 'Machine') -split ';') -contains $pf)) 'the PATH entry is gone'
    Assert-That (-not (Test-Path "$([Environment]::GetFolderPath('CommonPrograms'))\Kora.lnk")) 'the Start Menu shortcut is gone'
    Assert-That (-not (Test-Path "$pf\kora-desktop.exe")) 'the desktop app is gone'
    Assert-That (-not (Test-Path '\\.\pipe\kora-service')) 'no pipe'
    Assert-That (Test-Path "$srvA\.kora\request.json") 'the server folder was never touched'
    Write-Host 'all MSI checks passed'
}
catch {
    Write-Host "FAILED: $($_.Exception.Message)"
    Show-Diagnostics
    throw
}
finally {
    foreach ($u in $A, $B, $C) { Clear-TestAccount -Name $u }
}
