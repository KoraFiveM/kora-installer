# One-time setup of a Windows VM (Windows 11 or Server 2022/2025) as the self-hosted runner for the UAC job
# (.github/workflows/windows-uac.yml, label `kora-uac-vm`). Run it in Windows PowerShell 5.1 AS ADMINISTRATOR,
# then restart the VM and follow docs/windows-uac-ci.md (register the runner, run it INTERACTIVELY, snapshot).
#
# What it does:
#   * creates the runner account (a normal administrator: it gets a split token) and signs it in automatically;
#   * switches UAC on with the prompt on the normal desktop (where UI Automation can reach it);
#   * installs the prompt driver: a scheduled task "KoraUacDriver" that runs uac-answer.ps1 in the runner account's
#     desktop at HIGH integrity (UI Automation cannot drive the prompt, a SYSTEM process, from a medium process);
#   * stops the screen locking and the machine sleeping (a locked desktop has no prompt to answer).
#
# The VM must be a throw-away machine with no secrets: the job runs repository code as an administrator, the runner
# password is kept in the registry for the automatic sign-in, and the driver task lets any process of the runner
# account run uac-answer.ps1 at high integrity. Only same-repository pull requests and manual runs reach it
# (the workflow's condition); keep it that way, and revert to the snapshot after a suspicious run.
[CmdletBinding()]
param(
    [string]$RunnerUser = 'koraci',
    [Parameter(Mandatory)][securestring]$RunnerPassword
)
$ErrorActionPreference = 'Stop'
$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'run this as administrator' }

$plain = [System.Net.NetworkCredential]::new('', $RunnerPassword).Password

# 1. The runner account.
& net.exe user $RunnerUser $plain /add /expires:never /passwordchg:no 2>&1 | Out-Host
& net.exe localgroup Administrators $RunnerUser /add 2>&1 | Out-Host

# 2. Automatic sign-in (a plain password in the registry: acceptable only on this disposable VM; Sysinternals
#    Autologon stores it as an LSA secret instead, use that if you prefer).
$wl = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
Set-ItemProperty -Path $wl -Name AutoAdminLogon -Value '1'
Set-ItemProperty -Path $wl -Name DefaultUserName -Value $RunnerUser
Set-ItemProperty -Path $wl -Name DefaultDomainName -Value $env:COMPUTERNAME
Set-ItemProperty -Path $wl -Name DefaultPassword -Value $plain

# 3. UAC: on, the default behaviour for administrators (consent for non-Windows programs), prompt on the normal desktop.
$sys = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System'
Set-ItemProperty -Path $sys -Name EnableLUA -Value 1 -Type DWord
Set-ItemProperty -Path $sys -Name ConsentPromptBehaviorAdmin -Value 5 -Type DWord
Set-ItemProperty -Path $sys -Name ConsentPromptBehaviorUser -Value 3 -Type DWord
Set-ItemProperty -Path $sys -Name PromptOnSecureDesktop -Value 0 -Type DWord
Set-ItemProperty -Path $sys -Name FilterAdministratorToken -Value 0 -Type DWord

# 4. No lock screen, no screen saver, no sleep.
$pers = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\Personalization'
if (-not (Test-Path $pers)) { $null = New-Item -Path $pers -Force }
Set-ItemProperty -Path $pers -Name NoLockScreen -Value 1 -Type DWord
& powercfg.exe /change monitor-timeout-ac 0
& powercfg.exe /change standby-timeout-ac 0
& powercfg.exe /hibernate off

# 5. The prompt driver.
$dir = 'C:\kora-uac-driver'
$null = New-Item -ItemType Directory -Force -Path $dir
Copy-Item (Join-Path $PSScriptRoot 'uac-answer.ps1') (Join-Path $dir 'uac-answer.ps1') -Force
@'
$a = Get-Content -Raw -Path (Join-Path $PSScriptRoot 'args.json') | ConvertFrom-Json
& (Join-Path $PSScriptRoot 'uac-answer.ps1') -Answer $a.answer -ResultFile $a.resultFile -TimeoutSec $a.timeoutSec -UserName $a.userName -SecretFile $a.secretFile
'@ | Set-Content -Path (Join-Path $dir 'run.ps1') -Encoding ASCII
# The runner account may write the arguments and the fresh uac-answer.ps1 (the job copies it in), read nothing else.
& icacls.exe $dir /grant "${RunnerUser}:(OI)(CI)M" | Out-Host
$action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -ExecutionPolicy Bypass -File `"$dir\run.ps1`""
$taskPrincipal = New-ScheduledTaskPrincipal -UserId "$env:COMPUTERNAME\$RunnerUser" -LogonType Interactive -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 10) -MultipleInstances Parallel
Register-ScheduledTask -TaskName 'KoraUacDriver' -Action $action -Principal $taskPrincipal -Settings $settings -Force | Out-Null
Write-Host 'registered the scheduled task KoraUacDriver'

Write-Host ''
Write-Host 'Done. Restart the VM now (the UAC setting needs it). After the restart the VM signs in as'
Write-Host "$RunnerUser. Then follow docs/windows-uac-ci.md: register the runner (label kora-uac-vm), start it in a"
Write-Host 'normal (not elevated) window with run.cmd, put run.cmd in the Startup folder, and take a snapshot.'
