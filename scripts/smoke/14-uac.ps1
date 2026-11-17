# The panel's REAL administrator path on Windows, as far as the machine allows (see docs/windows-uac-ci.md).
#
# Where 13-admin-worker.ps1 starts the worker directly (`--selftest`, already elevated, no prompt), this script
# goes through the panel's own code: an unelevated `kora panel` (started as a filtered administrator token, or as a
# separate local account) is asked over its HTTP API to update itself from a folder that only administrators can
# write to (POST /api/v1/self-update); the panel then runs ShellExecute "runas" for `kora admin-worker`, the worker
# opens its pipe, both ends check each other, the update runs elevated and the answer comes back as a job result.
# Nothing on the machine is changed beyond a throw-away copy of kora.exe under Program Files.
#
# Modes
#   Hosted  windows-latest. The job's account is an ELEVATED administrator and UAC is normally off (EnableLUA=0,
#           and changing that needs a restart the job cannot survive), so there is no real split token and no
#           prompt. The scenarios that need them are SKIPPED with a notice; the ones that still say something are
#           run: an unelevated panel (a filtered token made by hand) must NOT be able to become an elevated worker,
#           and a standard account must not either.
#   Vm      a self-hosted Windows VM whose runner runs as a normal administrator in an interactive desktop session
#           with UAC on (scripts/smoke/vm-setup-uac-runner.ps1; docs/windows-uac-ci.md). The real prompt is answered
#           through UI Automation (uac-answer.ps1): Yes, No, nobody, and a standard account typing an
#           administrator's credentials.
#
# Maps to docs/windows-test-checklist.md: A2 (No, Yes), A3 (unanswered), A5 (standard account), W6 (the pipe is
# closed to the same user unelevated), W10 (the worker follows its panel's end), A11/W3-ish (worker seen and gone).
# What stays manual is listed in docs/windows-uac-ci.md.
[CmdletBinding()]
param(
    [ValidateSet('Hosted', 'Vm')][string]$Mode = 'Hosted',
    # Include the 5-minute "nobody answers" scenario (A3). Always on in the VM job, off in PR runs.
    [switch]$Slow
)
. "$PSScriptRoot/lib-accounts.ps1"

$kora = Get-KoraExe
$results = New-Object System.Collections.Generic.List[object]
$script:portNext = 47311

function Get-FreePort { $script:portNext++; $script:portNext }

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

# ---- observing the elevated side while a job runs --------------------------------------------------------------

# "Can an unelevated process of this user open the worker's pipe?" `denied` is the right answer. The probe runs
# under a filtered token when this process is elevated, directly when it is not.
function Test-PipeAsUnelevated([string]$PipeName) {
    $out = Join-Path $workDir ("probe-" + [guid]::NewGuid().ToString('N') + '.txt')
    $code = @"
try {
    `$c = New-Object System.IO.Pipes.NamedPipeClientStream('.', '$PipeName', [System.IO.Pipes.PipeDirection]::InOut)
    `$c.Connect(1500)
    'connected' | Set-Content '$out'
}
catch [UnauthorizedAccessException] { 'denied' | Set-Content '$out' }
catch [TimeoutException] { 'timeout' | Set-Content '$out' }
catch { ('error: ' + `$_.Exception.Message) | Set-Content '$out' }
"@
    $ps1 = [IO.Path]::ChangeExtension($out, '.ps1')
    Set-Content -Path $ps1 -Value $code -Encoding ASCII
    $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $argText = "-NoProfile -ExecutionPolicy Bypass -File `"$ps1`""
    if ([KoraTok]::IsElevated(0)) { $procId = [KoraTok]::LaunchFiltered($ps, "`"$ps`" $argText", $workDir) }
    else { $procId = (Start-Process -FilePath $ps -ArgumentList $argText -PassThru -WindowStyle Hidden).Id }
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ((Get-Process -Id $procId -ErrorAction SilentlyContinue) -and $sw.Elapsed.TotalSeconds -lt 20) { Start-Sleep -Milliseconds 200 }
    if (Test-Path $out) { (Get-Content $out -Raw).Trim() } else { 'no answer from the probe' }
}

# Start the elevated update through the panel and watch what happens until the job ends.
# Returns Post (the HTTP answer), Job (the job object or $null), Seen (what was observed).
function Invoke-ElevatedJob($Panel, [int]$TimeoutSec = 150, [scriptblock]$AfterPost = $null) {
    $seen = @{ Worker = $false; WorkerPid = 0; WorkerToken = ''; Pipe = $false; PipeName = ''; PipeProbe = ''; Consent = $false; Last = 0 }
    $post = Invoke-PanelApi -Panel $Panel -Method 'POST' -Path '/self-update' -Body @{}
    Write-Host "POST /self-update -> $($post.Status) $($post.Raw)"
    if ($post.Status -ne 202 -or -not $post.Body -or -not $post.Body.job) { return [pscustomobject]@{ Post = $post; Job = $null; Seen = $seen } }
    $id = [string]$post.Body.job
    if ($AfterPost) { & $AfterPost }
    $tick = {
        $now = [Environment]::TickCount
        if ($now - $seen.Last -lt 400) { return }
        $seen.Last = $now
        $w = @(Get-CimInstance Win32_Process -Filter "Name='kora.exe'" | Where-Object { $_.CommandLine -match 'admin-worker' })
        if ($w.Count -gt 0 -and -not $seen.Worker) {
            $seen.Worker = $true
            $seen.WorkerPid = [int]$w[0].ProcessId
            try { $f = Get-TokenFact -ProcessId $seen.WorkerPid; $seen.WorkerToken = "elevated=$($f.Elevated) type=$($f.ElevationType) integrity=$($f.Integrity)" }
            catch { $seen.WorkerToken = 'unreadable: ' + $_.Exception.Message }
        }
        $pipe = $null
        try { $pipe = [System.IO.Directory]::GetFiles('\\.\pipe\') | Where-Object { $_ -match 'kora-admin-' } | Select-Object -First 1 } catch { Write-Verbose "pipe list: $($_.Exception.Message)" }
        if ($pipe -and -not $seen.Pipe) {
            $seen.Pipe = $true
            $seen.PipeName = $pipe -replace '^\\\\\.\\pipe\\', ''
            $seen.PipeProbe = Test-PipeAsUnelevated $seen.PipeName
        }
        if (Get-Process -Name consent -ErrorAction SilentlyContinue) { $seen.Consent = $true }
    }
    $job = Wait-PanelJob -Panel $Panel -JobId $id -TimeoutSec $TimeoutSec -OnTick $tick
    if ($job) { Write-Host ('job ended: state={0} error={1} result={2}' -f $job.state, ($job.error | ConvertTo-Json -Compress), ($job.result | ConvertTo-Json -Compress -Depth 4)) }
    [pscustomobject]@{ Post = $post; Job = $job; Seen = $seen }
}

function Assert-NoWorkerLeft([int]$WithinSec = 30) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    do {
        $left = @(Get-KoraProcess | Where-Object { $_.CommandLine -match 'admin-worker|admin-step' })
        if ($left.Count -eq 0) { Assert-That $true 'no kora admin-worker or admin-step is left running'; return }
        Start-Sleep -Milliseconds 500
    } while ($sw.Elapsed.TotalSeconds -lt $WithinSec)
    throw "a worker was left running after $WithinSec s: $($left | Format-Table -AutoSize | Out-String)"
}

function Assert-PanelAlive($Panel) {
    $r = Invoke-PanelApi -Panel $Panel -Method 'GET' -Path '/meta'
    Assert-That ($r.Status -eq 200) "the panel still answers (GET /meta -> $($r.Status))"
}

function Assert-Unelevated($Panel, [string]$Why) {
    try {
        $f = Get-TokenFact -ProcessId $Panel.PanelProcessId
        Assert-That (-not $f.Elevated) "the panel is not elevated ($Why): type $($f.ElevationType), integrity $($f.Integrity)"
        Assert-That ($f.Integrity -lt 12288) 'the panel runs below high integrity'
    }
    catch [System.ComponentModel.Win32Exception] {
        Write-Host "::warning::cannot read the panel's token to check it is unelevated: $($_.Exception.Message)"
    }
}

# ---- the VM's prompt driver ---------------------------------------------------------------------------------------

$driverDir = 'C:\kora-uac-driver'
function Invoke-UacDriver([ValidateSet('Yes', 'No', 'Credentials', 'None', 'Dump')][string]$Answer, [int]$TimeoutSec = 60, [string]$UserName = '', [string]$SecretFile = '') {
    Copy-Item (Join-Path $PSScriptRoot 'uac-answer.ps1') (Join-Path $driverDir 'uac-answer.ps1') -Force
    $res = Join-Path $workDir ("driver-" + [guid]::NewGuid().ToString('N') + '.json')
    @{ answer = $Answer; resultFile = $res; timeoutSec = $TimeoutSec; userName = $UserName; secretFile = $SecretFile } | ConvertTo-Json | Set-Content -Path (Join-Path $driverDir 'args.json') -Encoding ASCII
    & schtasks.exe /run /tn KoraUacDriver | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'schtasks /run KoraUacDriver failed: is the VM set up (vm-setup-uac-runner.ps1)?' }
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

# ---------------------------------------------------------------------------------------------------------------
# Preflight
# ---------------------------------------------------------------------------------------------------------------
Write-Host "mode: $Mode"
$policy = Get-UacPolicy
Write-Host ($policy | Format-List | Out-String)
$luaOn = ($policy.EnableLUA -eq 1)
$me = Get-TokenFact
Write-TokenDiagnostic 'the job (this process)'
$elevatedJob = $me.Elevated
Write-Host "UAC on (EnableLUA=1): $luaOn; the job is elevated: $elevatedJob; elevation type: $($me.ElevationType)"
if ($Mode -eq 'Hosted') { Assert-That $elevatedJob 'the hosted job is an elevated administrator (as documented)' }
if ($Mode -eq 'Vm') {
    Assert-That $luaOn 'the VM has UAC switched on (EnableLUA=1)'
    Assert-That (-not $elevatedJob -and $me.ElevationType -eq 3) 'the VM job runs as a normal administrator (unelevated half of a split token); see docs/windows-uac-ci.md'
    Assert-That (Test-Path (Join-Path $driverDir 'run.ps1')) "the prompt driver is installed in $driverDir"
}
$splitPossible = $luaOn
$policyBefore = Get-UacPolicy
$workDir = Initialize-UacWorkDir
$exe = Initialize-ProtectedKora -Source $kora
$panels = New-Object System.Collections.Generic.List[object]
$adm = $null; $std = $null

# Hosted: the job is elevated, so the unelevated panel is a filtered copy of the job's own token. VM: the job is
# unelevated already.
$ownKind = if ($Mode -eq 'Vm') { 'Self' } else { 'Filtered' }

function Open-Panel([string]$Kind, [string]$Tag, [pscredential]$Credential) {
    $p = Open-KoraPanel -Kind $Kind -Exe $exe -Dir $workDir -Tag $Tag -Port (Get-FreePort) -Credential $Credential
    $panels.Add($p)
    $p
}

try {
    $adm = Initialize-TestAccount -Name 'koraadm' -Administrator
    $std = Initialize-TestAccount -Name 'korastd'

    # -----------------------------------------------------------------------------------------------------------
    Invoke-Scenario 'S1 an unelevated administrator token cannot become the worker (same user, filtered)' {
        # With UAC off nothing can elevate: the "runas" starts the worker with the same filtered token, which must
        # not be able to serve (its pipe wants the ENABLED Administrators group). With UAC on and the policy
        # "elevate without prompting", a hand-made token has no linked elevated half, so Windows may refuse or may
        # elevate: both are recorded, and only the safety properties are asserted.
        $before = Initialize-UacPolicy -ConsentPromptBehaviorAdmin 0
        try {
            $p = Open-Panel -Kind $ownKind -Tag 's1'
            Assert-Unelevated $p 'a filtered administrator token'
            $r = Invoke-ElevatedJob $p -TimeoutSec 180
            Assert-That ($null -ne $r.Job) 'the job ended instead of hanging'
            Write-Host "observed: $($r.Seen | ConvertTo-Json -Compress)"
            if (-not $luaOn) {
                Assert-That ($r.Job.state -eq 'failed') 'with UAC off an unelevated panel gets no administrator worker: the job fails (it must not succeed)'
                if ($r.Seen.Pipe) { Assert-That ($r.Seen.PipeProbe -ne 'connected') 'the pipe was not open to an unelevated process' }
            }
            else {
                Write-Host "::notice::S1 with UAC on: job state '$($r.Job.state)'; record this in docs/windows-uac-ci.md once seen"
            }
            Assert-NoWorkerLeft
            Assert-PanelAlive $p
        }
        finally { Restore-UacPolicy $before }
    }

    # -----------------------------------------------------------------------------------------------------------
    Invoke-Scenario 'S2 split token, the elevation approved by policy (A1-like, W6, A11)' {
        $before = Initialize-UacPolicy -ConsentPromptBehaviorAdmin 0
        try {
            # Hosted: the other administrator through the secondary logon service (a split token when UAC is on).
            # VM: this very (unelevated) runner account.
            if ($Mode -eq 'Vm') { $p = Open-Panel -Kind 'Self' -Tag 's2' } else { $p = Open-Panel -Kind 'Account' -Tag 's2' -Credential $adm.Credential }
            Assert-Unelevated $p 'an administrator with a split token'
            $r = Invoke-ElevatedJob $p -TimeoutSec 240
            Write-Host "observed: $($r.Seen | ConvertTo-Json -Compress)"
            Assert-That ($r.Post.Status -eq 202) 'the panel accepted the update job (it needs administrator rights there)'
            Assert-That ($null -ne $r.Job -and $r.Job.state -eq 'done') 'the elevated update finished through the worker (job done)'
            if ($r.Seen.Worker) {
                Assert-That ($r.Seen.WorkerToken -match 'elevated=True') "the worker really was elevated ($($r.Seen.WorkerToken))"
            }
            else { Write-Host '::warning::the worker was too short-lived to be observed (not a failure by itself)' }
            if ($r.Seen.Pipe) { Assert-That ($r.Seen.PipeProbe -eq 'denied') "W6: an unelevated process of the same user is refused at the pipe (got: $($r.Seen.PipeProbe))" }
            else { Write-Host '::warning::the pipe was not seen while it existed, so W6 was not exercised this run' }
            Assert-NoWorkerLeft
            Assert-PanelAlive $p
        }
        finally { Restore-UacPolicy $before }
    } -SkipReason $(if (-not $splitPossible) { 'UAC is switched off on this machine (EnableLUA=0), so there is no split token and no elevation to approve. Changing it needs a restart; use the self-hosted VM job.' } else { '' })

    # -----------------------------------------------------------------------------------------------------------
    Invoke-Scenario 'S2b the panel ends while the worker runs: the worker goes too (W10)' {
        $before = Initialize-UacPolicy -ConsentPromptBehaviorAdmin 0
        try {
            if ($Mode -eq 'Vm') { $p = Open-Panel -Kind 'Self' -Tag 's2b' } else { $p = Open-Panel -Kind 'Account' -Tag 's2b' -Credential $adm.Credential }
            $killed = $false
            $post = Invoke-PanelApi -Panel $p -Method 'POST' -Path '/self-update' -Body @{}
            Assert-That ($post.Status -eq 202) "the job started ($($post.Status))"
            $sw = [Diagnostics.Stopwatch]::StartNew()
            while ($sw.Elapsed.TotalSeconds -lt 60 -and -not $killed) {
                $w = @(Get-CimInstance Win32_Process -Filter "Name='kora.exe'" | Where-Object { $_.CommandLine -match 'admin-worker' })
                if ($w.Count -gt 0) { Stop-Process -Id $p.PanelProcessId -Force; $killed = $true }
                else { Start-Sleep -Milliseconds 100 }
            }
            if (-not $killed) { throw 'no worker appeared within 60 s, so there was nothing to follow (check S2 first)' }
            Assert-NoWorkerLeft -WithinSec 30
        }
        finally { Restore-UacPolicy $before }
    } -SkipReason $(if (-not $splitPossible) { 'needs a working elevation (UAC on); see S2' } else { '' })

    # -----------------------------------------------------------------------------------------------------------
    Invoke-Scenario 'S4 a standard (non-administrator) account is refused when the policy denies elevation (A5, refusal side)' {
        $before = Initialize-UacPolicy -ConsentPromptBehaviorUser 0
        try {
            $p = Open-Panel -Kind 'Account' -Tag 's4' -Credential $std.Credential
            Assert-Unelevated $p 'a standard account'
            $f = Get-TokenFact -ProcessId $p.PanelProcessId
            Assert-That ($f.ElevationType -eq 1) "a standard account has no split token (type $($f.ElevationType))"
            $r = Invoke-ElevatedJob $p -TimeoutSec 180
            Write-Host "observed: $($r.Seen | ConvertTo-Json -Compress)"
            Assert-That ($null -ne $r.Job -or $r.Post.Status -ge 400) 'the request ended with an answer instead of hanging'
            if ($r.Job) { Assert-That ($r.Job.state -ne 'done') 'a standard account never gets an administrator update (the job must not finish)' }
            Assert-NoWorkerLeft
            Assert-PanelAlive $p
        }
        finally { Restore-UacPolicy $before }
    }

    # -----------------------------------------------------------------------------------------------------------
    # The real prompt (VM only).
    $needVm = if ($Mode -ne 'Vm') { 'answering the real prompt needs the interactive self-hosted VM (docs/windows-uac-ci.md)' } else { '' }

    Invoke-Scenario 'S3a the real prompt, answer Yes (A1, A2)' {
        $before = Initialize-UacPolicy -ConsentPromptBehaviorAdmin 5 -PromptOnSecureDesktop 0
        try {
            $p = Open-Panel -Kind 'Self' -Tag 's3a'
            $r = Invoke-ElevatedJob $p -TimeoutSec 180 -AfterPost { $script:drv = Invoke-UacDriver -Answer 'Yes' -TimeoutSec 60 }
            $d = Wait-UacDriver -ResultFile $script:drv
            Assert-That ($d.found -and $d.answered) 'the prompt appeared and was answered Yes'
            Assert-That ($r.Seen.Consent) 'consent.exe was seen'
            Assert-That ($null -ne $r.Job -and $r.Job.state -eq 'done') 'after Yes the job finishes'
            Assert-NoWorkerLeft
        }
        finally { Restore-UacPolicy $before }
    } -SkipReason $needVm

    Invoke-Scenario 'S3b the real prompt, answer No (A2)' {
        $before = Initialize-UacPolicy -ConsentPromptBehaviorAdmin 5 -PromptOnSecureDesktop 0
        try {
            $p = Open-Panel -Kind 'Self' -Tag 's3b'
            $r = Invoke-ElevatedJob $p -TimeoutSec 180 -AfterPost { $script:drv = Invoke-UacDriver -Answer 'No' -TimeoutSec 60 }
            $d = Wait-UacDriver -ResultFile $script:drv
            Assert-That ($d.found -and $d.answered) 'the prompt appeared and was answered No'
            Assert-That ($null -ne $r.Job -and $r.Job.state -eq 'failed') 'after No the job fails'
            $text = ($r.Job | ConvertTo-Json -Depth 6 -Compress)
            Assert-That ($text -match 'admin\.declined|did not get permission|declined') 'and says Windows did not get permission (the guided answer, not a raw error)'
            Assert-NoWorkerLeft
            Assert-PanelAlive $p
            # Ask again, answer Yes: it finishes.
            $r2 = Invoke-ElevatedJob $p -TimeoutSec 180 -AfterPost { $script:drv = Invoke-UacDriver -Answer 'Yes' -TimeoutSec 60 }
            $null = Wait-UacDriver -ResultFile $script:drv
            Assert-That ($null -ne $r2.Job -and $r2.Job.state -eq 'done') 'asking again and answering Yes finishes'
        }
        finally { Restore-UacPolicy $before }
    } -SkipReason $needVm

    Invoke-Scenario 'S3c the real prompt left unanswered for 5 minutes; answering the stale prompt afterwards starts nothing (A3)' {
        $before = Initialize-UacPolicy -ConsentPromptBehaviorAdmin 5 -PromptOnSecureDesktop 0
        try {
            $p = Open-Panel -Kind 'Self' -Tag 's3c'
            $r = Invoke-ElevatedJob $p -TimeoutSec 420 -AfterPost { $script:drv = Invoke-UacDriver -Answer 'None' -TimeoutSec 60 }
            $null = Wait-UacDriver -ResultFile $script:drv
            Assert-That ($null -ne $r.Job -and $r.Job.state -eq 'failed') 'with nobody answering the job fails'
            Assert-That ((($r.Job | ConvertTo-Json -Depth 6 -Compress) -match 'timed_out|Nobody answered|nobody')) 'and says nobody answered'
            $late = Invoke-UacDriver -Answer 'Yes' -TimeoutSec 30
            $null = Wait-UacDriver -ResultFile $late -TimeoutSec 60
            Start-Sleep -Seconds 10
            Assert-NoWorkerLeft
        }
        finally { Restore-UacPolicy $before }
    } -SkipReason $(if ($needVm) { $needVm } elseif (-not $Slow) { 'takes over 5 minutes: run with -Slow (always on in the VM job)' } else { '' })

    Invoke-Scenario 'S5 standard account, an administrator answers with their own credentials (A5, W5)' {
        $before = Initialize-UacPolicy -ConsentPromptBehaviorUser 1 -PromptOnSecureDesktop 0
        $secretFile = Join-Path $driverDir 'secret.txt'
        try {
            Set-Content -Path $secretFile -Value (Get-SecretText $adm.Credential.Password) -Encoding ASCII -NoNewline
            & icacls.exe $secretFile /inheritance:r /grant:r "${env:USERNAME}:(R)" | Out-Null
            $p = Open-Panel -Kind 'Account' -Tag 's5' -Credential $std.Credential
            $r = Invoke-ElevatedJob $p -TimeoutSec 180 -AfterPost { $script:drv = Invoke-UacDriver -Answer 'Credentials' -TimeoutSec 60 -UserName $adm.Name -SecretFile $secretFile }
            $d = Wait-UacDriver -ResultFile $script:drv
            Assert-That ($d.found -and $d.answered) 'the credential prompt appeared and was filled in'
            Assert-That ($null -ne $r.Job -and $r.Job.state -eq 'done') 'with the administrator''s credentials the standard account''s update finishes'
            Assert-NoWorkerLeft
        }
        finally {
            Remove-Item $secretFile -Force -ErrorAction SilentlyContinue
            Restore-UacPolicy $before
        }
    } -SkipReason $needVm
}
finally {
    foreach ($p in $panels) { Close-KoraPanel $p }
    Restore-UacPolicy $policyBefore
    if ($adm) { Clear-TestAccount -Name $adm.Name }
    if ($std) { Clear-TestAccount -Name $std.Name }
    Remove-Item (Split-Path $exe -Parent) -Recurse -Force -ErrorAction SilentlyContinue
}

# ---------------------------------------------------------------------------------------------------------------
Write-Host ''
Write-Host '=== UAC scenarios ==='
$table = $results | Format-Table -AutoSize -Wrap | Out-String -Width 250
Write-Host $table
if ($env:GITHUB_STEP_SUMMARY) {
    $md = @("### Windows UAC job ($Mode; UAC on: $luaOn)", '', '| Scenario | Outcome | Note |', '|---|---|---|')
    foreach ($r in $results) { $md += ('| {0} | {1} | {2} |' -f $r.Name, $r.Outcome, ($r.Note -replace '\|', '/' -replace '\r?\n', ' ')) }
    Add-Content -Path $env:GITHUB_STEP_SUMMARY -Value ($md -join "`n")
}
$failed = @($results | Where-Object { $_.Outcome -eq 'FAILED' })
if ($failed.Count -gt 0) { throw "$($failed.Count) UAC scenario(s) failed" }
exit 0
