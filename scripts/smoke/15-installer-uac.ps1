# What still shows a Windows permission (UAC) prompt, and what the administrator service refuses (see
# docs/windows-uac-ci.md). There is exactly one prompt left in Kora: the installer's. The panel itself never asks.
#
#   * the installer: `install.ps1` runs the checked MSI with ONE prompt (Start-Process -Verb RunAs), and
#     `kora service install` does the same through its own download (not testable here: it needs a published release;
#     install.ps1 exercises the same msiexec step against a local release folder);
#   * the service's refusals, by identity (a token SID read from the pipe connection): an account that is not on
#     the list, an unelevated token of an administrator who is not on the list, and the same token once allowed.
#
# Modes
#   Hosted  windows-latest. An ELEVATED administrator with UAC normally off: the installer's prompt does not appear
#           (Start-Process -Verb RunAs just runs), so only what it installs is checked; the refusals run in full.
#   Vm      a self-hosted VM with UAC on and an interactive desktop (scripts/smoke/vm-setup-uac-runner.ps1). The real
#           prompt of the installer is answered through UI Automation (uac-answer.ps1): Yes, and No (then install.ps1
#           -System must fail without installing anything and without touching a per-user install).
#
# Maps to docs/windows-test-checklist.md S1 (one prompt), S3 (accounts), S8 (without the service).
[CmdletBinding()]
param(
    [ValidateSet('Hosted', 'Vm')][string]$Mode = 'Hosted',
    # The MSI to install (built by the workflow). Required.
    [Parameter(Mandatory)][string]$Msi
)
. "$PSScriptRoot/lib-accounts.ps1"

$kora = Get-KoraExe
$Msi = (Resolve-Path $Msi).Path
$results = New-Object System.Collections.Generic.List[object]
$pf = [Environment]::GetFolderPath('ProgramFiles') + '\Kora'
$koraSvc = "$pf\kora.exe"

function Invoke-Scenario([string]$Name, [scriptblock]$Body, [string]$SkipReason = '') {
    if ($SkipReason) {
        Write-Host "::notice title=UAC scenario skipped::${Name}: $SkipReason"
        $results.Add([pscustomobject]@{ Name = $Name; Outcome = 'skipped'; Note = $SkipReason })
        return
    }
    Write-Host "::group::SCENARIO $Name"
    $err = $null
    try { & $Body }
    catch { $err = $_ }
    Write-Host '::endgroup::'
    if ($null -eq $err) {
        $results.Add([pscustomobject]@{ Name = $Name; Outcome = 'passed'; Note = '' })
        Write-Host "PASSED: $Name"
    }
    else {
        $msg = $err.Exception.Message
        $results.Add([pscustomobject]@{ Name = $Name; Outcome = 'FAILED'; Note = $msg })
        Write-Host "::error title=UAC scenario failed::${Name}: $msg"
        Write-Host $err.ScriptStackTrace
        Write-SystemDiagnostic "diagnostics after the failure of: $Name"
    }
}

# ---- the VM's prompt driver ---------------------------------------------------------------------------------------

$driverDir = 'C:\kora-uac-driver'
function Invoke-UacDriver([ValidateSet('Yes', 'No', 'Credentials', 'None', 'Dump')][string]$Answer, [int]$TimeoutSec = 60, [string]$UserName = '', [string]$SecretFile = '') {
    Copy-Item (Join-Path $PSScriptRoot 'uac-answer.ps1') (Join-Path $driverDir 'uac-answer.ps1') -Force
    $res = Join-Path $workDir ("driver-" + [guid]::NewGuid().ToString('N') + '.json')
    @{ answer = $Answer; resultFile = $res; timeoutSec = $TimeoutSec; userName = $UserName; secretFile = $SecretFile } | ConvertTo-Json | Set-Content -Path (Join-Path $driverDir 'args.json') -Encoding ASCII
    $t = Invoke-Native 'schtasks.exe' @('/run', '/tn', 'KoraUacDriver')
    Write-Host $t.Output
    if ($t.Code -ne 0) { throw 'schtasks /run KoraUacDriver failed: is the VM set up (vm-setup-uac-runner.ps1)?' }
    $res
}

function Wait-UacDriver([string]$ResultFile, [int]$TimeoutSec = 90) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path $ResultFile) -and $sw.Elapsed.TotalSeconds -lt $TimeoutSec) { Start-Sleep -Milliseconds 300 }
    if (-not (Test-Path $ResultFile)) { throw "the prompt driver wrote no result within $TimeoutSec s" }
    $r = Get-Content $ResultFile -Raw | ConvertFrom-Json
    Write-Host ("driver: found={0} answered={1} button='{2}' buttons=[{3}] error='{4}'" -f $r.found, $r.answered, $r.button, ($r.buttons -join ', '), $r.error)
    if ($r.tree) { Write-Host ($r.tree -join "`n") }
    $r
}

# ---- a local release folder for install.ps1 (it accepts http only for localhost) -------------------------------------
function Open-ReleaseServer([string]$Dir, [int]$Port) {
    $py = (Get-Command python -ErrorAction SilentlyContinue)
    if (-not $py) { $py = Get-Command python3 -ErrorAction Stop }
    Start-Process -FilePath $py.Source -ArgumentList @('-m', 'http.server', "$Port", '--bind', '127.0.0.1', '--directory', $Dir) -PassThru -WindowStyle Hidden
}

function Assert-ServiceGone {
    Assert-That (-not (Get-Service KoraService -ErrorAction SilentlyContinue)) 'KoraService is not installed'
    Assert-That (-not (Test-Path $koraSvc)) 'nothing was put in Program Files\Kora'
}

function Invoke-KoraMsiRemoval {
    $p = Start-Process msiexec.exe -ArgumentList "/x `"$Msi`" /qn" -Wait -PassThru
    Write-Host "msiexec /x -> $($p.ExitCode)"
}

# ---------------------------------------------------------------------------------------------------------------
Write-Host "mode: $Mode"
$policy = Get-UacPolicy
Write-Host ($policy | Format-List | Out-String)
$luaOn = ($policy.EnableLUA -eq 1)
$me = Get-TokenFact
Write-TokenDiagnostic 'the job (this process)'
if ($Mode -eq 'Hosted') { Assert-That $me.Elevated 'the hosted job is an elevated administrator (as documented)' }
if ($Mode -eq 'Vm') {
    Assert-That $luaOn 'the VM has UAC switched on (EnableLUA=1)'
    Assert-That (-not $me.Elevated -and $me.ElevationType -eq 3) 'the VM job runs as a normal administrator (unelevated half of a split token)'
    Assert-That (Test-Path (Join-Path $driverDir 'run.ps1')) "the prompt driver is installed in $driverDir"
}
$policyBefore = Get-UacPolicy
$workDir = Initialize-UacWorkDir
$client = (Resolve-Path "$PSScriptRoot/svc-client.ps1").Path
$rel = Join-Path $workDir 'release'
$null = New-Item -ItemType Directory -Force -Path $rel
Copy-Item $kora (Join-Path $rel 'kora-windows-x86_64.exe')
Copy-Item $Msi (Join-Path $rel 'kora-windows-x86_64.msi')
foreach ($f in 'kora-windows-x86_64.exe', 'kora-windows-x86_64.msi') {
    $h = (Get-FileHash -Algorithm SHA256 (Join-Path $rel $f)).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText((Join-Path $rel "$f.sha256"), "$h  $f`n")
}
$port = 47411
$server = Open-ReleaseServer -Dir $rel -Port $port
Start-Sleep -Seconds 2
$base = "http://127.0.0.1:$port"
$installPs1 = (Resolve-Path "$PSScriptRoot/../install.ps1").Path
$adm = $null; $std = $null
$shell = if ($Mode -eq 'Vm') { 'powershell.exe' } else { 'pwsh.exe' }

try {
    $adm = Initialize-TestAccount -Name 'koraadm' -Administrator
    $std = Initialize-TestAccount -Name 'korastd'
    $stdSid = $std.Sid

    if ($Mode -eq 'Vm') {
        Invoke-Scenario 'P1 the installer asks ONCE, the answer is No: nothing is installed (S1, refusal side)' {
            $before = Initialize-UacPolicy -ConsentPromptBehaviorAdmin 5 -PromptOnSecureDesktop 0
            try {
                $script:drv = $null
                $job = Start-Job -ScriptBlock { & $using:shell -NoProfile -ExecutionPolicy Bypass -File $using:installPs1 -Base $using:base -System -NoLaunch -NoShortcuts 2>&1; $LASTEXITCODE }
                Start-Sleep -Seconds 8
                $script:drv = Invoke-UacDriver -Answer 'No' -TimeoutSec 90
                $null = Wait-Job $job -Timeout 180
                Write-Host (Receive-Job $job | Out-String)
                $d = Wait-UacDriver -ResultFile $script:drv
                Assert-That ($d.found -and $d.answered) 'the installer prompt appeared and was answered No'
                Assert-ServiceGone
            }
            finally { Restore-UacPolicy $before }
        }
        Invoke-Scenario 'P2 the installer asks ONCE, the answer is Yes: the service is installed (S1)' {
            $before = Initialize-UacPolicy -ConsentPromptBehaviorAdmin 5 -PromptOnSecureDesktop 0
            try {
                $job = Start-Job -ScriptBlock { & $using:shell -NoProfile -ExecutionPolicy Bypass -File $using:installPs1 -Base $using:base -System -NoLaunch -NoShortcuts 2>&1; $LASTEXITCODE }
                Start-Sleep -Seconds 8
                $script:drv = Invoke-UacDriver -Answer 'Yes' -TimeoutSec 90
                $null = Wait-Job $job -Timeout 300
                Write-Host (Receive-Job $job | Out-String)
                $d = Wait-UacDriver -ResultFile $script:drv
                Assert-That ($d.found -and $d.answered) 'the installer prompt appeared and was answered Yes'
                Assert-That ((Get-Service KoraService -ErrorAction Stop).Status -eq 'Running') 'KoraService is running'
                $prompts = @(Get-Process -Name consent -ErrorAction SilentlyContinue).Count
                Assert-That ($prompts -eq 0) 'no further prompt is showing'
            }
            finally { Restore-UacPolicy $before }
        }
    }
    else {
        Invoke-Scenario 'P1 install.ps1 -System runs the checked MSI and installs the service (S1; no prompt on an elevated runner)' {
            $r = Invoke-Native $shell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $installPs1, '-Base', $base, '-System', '-NoLaunch', '-NoShortcuts')
            Write-Host $r.Output
            Assert-That ($r.Code -eq 0) "install.ps1 -System succeeded (exit $($r.Code))"
            Assert-That ((Get-Service KoraService -ErrorAction Stop).Status -eq 'Running') 'KoraService is running'
            Assert-That (Test-Path $koraSvc) 'kora.exe is in Program Files\Kora'
        }
    }

    # From here the service is installed (by P1/P2 above); the first allowed account is the installing one.
    Invoke-Scenario 'R1 an account that is not on the list is refused by the service' {
        $r = Invoke-AsAccount -Kind Account -Credential $std.Credential -Dir $workDir -Script "& '$koraSvc' service status"
        Write-Host $r.Output
        Assert-That ($r.Output -match 'serves this account: no') 'running, but not for this account'
        $c = Invoke-AsAccount -Kind Account -Credential $std.Credential -Dir $workDir -Script "& '$client' -Verb admin-step -Arguments run,--step,system.firewall,--dir,'$workDir'; exit `$LASTEXITCODE"
        Write-Host $c.Output
        Assert-That ($c.Code -eq 3 -or $c.Code -eq 5) "the request is refused or the pipe is closed to it (exit $($c.Code))"
    }

    Invoke-Scenario 'R2 an unelevated token of an administrator who is not on the list is refused; allowed, it is served; denied, refused again' {
        $kind = if ($Mode -eq 'Vm') { 'Self' } else { 'Filtered' }
        $st = { param($k) Invoke-AsAccount -Kind $k -Dir $workDir -Script "& '$koraSvc' service status" }
        $mySid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        $r = & $st $kind
        Write-Host $r.Output
        $wasAllowed = (& $koraSvc service list | Out-String) -match [regex]::Escape($mySid)
        if (-not $wasAllowed) { Assert-That ($r.Output -match 'serves this account: no') 'not on the list: refused (even though this is an administrator)' }
        & $koraSvc service allow $mySid | Out-Host
        $r = & $st $kind
        Write-Host $r.Output
        Assert-That ($r.Output -match 'serves this account: yes') 'allowed: served at medium integrity, nothing elevated'
        if (-not $wasAllowed) {
            & $koraSvc service deny $mySid | Out-Host
            $r = & $st $kind
            Assert-That ($r.Output -match 'serves this account: no') 'denied again: refused at once'
        }
    }

    Invoke-Scenario 'R3 the panel of an allowed standard account sees the service as ready; a not-allowed one sees not_allowed (no prompt either way)' {
        & $koraSvc service allow $stdSid | Out-Host
        $p = Open-KoraPanel -Kind 'Account' -Exe $koraSvc -Dir $workDir -Tag 'r3' -Port 47421 -Credential $std.Credential
        try {
            $s = Invoke-PanelApi -Panel $p -Method 'GET' -Path '/system'
            Write-Host $s.Raw
            Assert-That ($s.Body.admin_service.state -eq 'ready') 'allowed: admin_service ready'
            & $koraSvc service deny $stdSid | Out-Host
            $s = Invoke-PanelApi -Panel $p -Method 'GET' -Path '/system'
            Assert-That ($s.Body.admin_service.state -eq 'not_allowed') 'denied: admin_service not_allowed'
            Assert-That (-not (Get-Process -Name consent -ErrorAction SilentlyContinue)) 'and no permission prompt appeared'
        }
        finally { Close-KoraPanel $p }
    }
}
finally {
    if ($server) { Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue }
    Invoke-KoraMsiRemoval
    Restore-UacPolicy $policyBefore
    if ($adm) { Clear-TestAccount -Name $adm.Name }
    if ($std) { Clear-TestAccount -Name $std.Name }
}

Write-Host ''
Write-Host '=== installer / service UAC scenarios ==='
Write-Host ($results | Format-Table -AutoSize -Wrap | Out-String -Width 250)
if ($env:GITHUB_STEP_SUMMARY) {
    $md = @("### Windows installer and service UAC job ($Mode; UAC on: $luaOn)", '', '| Scenario | Outcome | Note |', '|---|---|---|')
    foreach ($r in $results) { $md += ('| {0} | {1} | {2} |' -f $r.Name, $r.Outcome, ($r.Note -replace '\|', '/' -replace '\r?\n', ' ')) }
    Add-Content -Path $env:GITHUB_STEP_SUMMARY -Value ($md -join "`n")
}
$failed = @($results | Where-Object { $_.Outcome -eq 'FAILED' })
if ($failed.Count -gt 0) { throw "$($failed.Count) scenario(s) failed" }
exit 0
