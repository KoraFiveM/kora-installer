# Answers a real Windows permission (UAC) prompt through UI Automation. VM variant only (see docs/windows-uac-ci.md).
#
# Must run in Windows PowerShell 5.1 (powershell.exe: UIAutomationClient is not in PowerShell 7), in the SAME
# interactive desktop as the prompt, and at HIGH integrity: the prompt (consent.exe) is a SYSTEM process, and
# Windows (UIPI) does not let a medium-integrity process drive a higher one. The VM setup registers a scheduled
# task "KoraUacDriver" (interactive, highest privileges, no password prompt) that runs this script with the
# arguments in C:\kora-uac\driver-args.json; the job starts it with `schtasks /run` (15-installer-uac.ps1: Invoke-UacDriver).
# The prompt must be on the normal desktop (policy PromptOnSecureDesktop=0), not the secure desktop.
#
#   -Answer Yes|No   click that button
#   -Answer Credentials   type -SecretFile's password for -UserName into the credential prompt, then OK/Yes
#   -Answer None     wait for the prompt and report it, answer nothing
#   -Answer Dump     wait for the prompt and write its whole UI tree (for adjusting this script)
# The result is written to -ResultFile as JSON: found, answered, button, buttons (all button names seen), tree (on
# failure or Dump), error.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Yes', 'No', 'Credentials', 'None', 'Dump')][string]$Answer,
    [Parameter(Mandatory)][string]$ResultFile,
    [int]$TimeoutSec = 60,
    [string]$UserName = '',
    [string]$SecretFile = ''
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms

$result = [ordered]@{ found = $false; answered = $false; button = ''; buttons = @(); tree = @(); error = '' }

function Save-Result([string]$Path) {
    ($result | ConvertTo-Json -Depth 6) | Set-Content -Path $Path -Encoding UTF8
}

function Find-PromptWindow {
    $ids = @(Get-Process -Name consent -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
    if ($ids.Count -eq 0) { return $null }
    foreach ($id in $ids) {
        $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $id)
        $w = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
        if ($w) { return $w }
    }
    $null
}

function Get-UiTree($Element, [int]$Depth = 0, [int]$Max = 6) {
    $out = @()
    $c = $Element.Current
    $out += ('{0}{1} | name="{2}" | id="{3}" | class="{4}"' -f ('  ' * $Depth), $c.ControlType.ProgrammaticName, $c.Name, $c.AutomationId, $c.ClassName)
    if ($Depth -lt $Max) {
        foreach ($k in $Element.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)) { $out += Get-UiTree -Element $k -Depth ($Depth + 1) -Max $Max }
    }
    $out
}

function Get-ButtonList($Window) {
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
    @($Window.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond))
}

function Use-Button($Window, [string[]]$Names, [string]$Accelerator) {
    foreach ($b in (Get-ButtonList $Window)) {
        if ($Names -contains $b.Current.Name) {
            $p = $null
            if ($b.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$p)) { $p.Invoke(); return $b.Current.Name }
        }
    }
    # Fallback: the accelerator key (Alt+Y / Alt+N in the English prompt) once the prompt has the focus.
    try { $Window.SetFocus() } catch { Write-Verbose "no focus: $($_.Exception.Message)" }
    [System.Windows.Forms.SendKeys]::SendWait($Accelerator)
    return "keys $Accelerator"
}

try {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $win = $null
    while (-not $win -and $sw.Elapsed.TotalSeconds -lt $TimeoutSec) {
        $win = Find-PromptWindow
        if (-not $win) { Start-Sleep -Milliseconds 250 }
    }
    if (-not $win) { $result.error = "no consent.exe window within $TimeoutSec s"; Save-Result -Path $ResultFile; exit 2 }
    $result.found = $true
    $result.buttons = @(Get-ButtonList $win | ForEach-Object { $_.Current.Name })
    Start-Sleep -Milliseconds 400   # let it finish drawing

    switch ($Answer) {
        'Yes' { $result.button = Use-Button -Window $win -Names @('Yes') -Accelerator '%y'; $result.answered = $true }
        'No' { $result.button = Use-Button -Window $win -Names @('No') -Accelerator '%n'; $result.answered = $true }
        'None' { }
        'Dump' { $result.tree = @(Get-UiTree $win) }
        'Credentials' {
            if (-not $SecretFile -or -not (Test-Path $SecretFile) -or -not $UserName) { throw '-UserName and -SecretFile are needed' }
            $secret = (Get-Content -Path $SecretFile -Raw).Trim()
            # The account list: pick the entry that names the user (it may already be selected).
            $items = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
            foreach ($i in $items) {
                if ($i.Current.Name -like "*$UserName*" -and $i.Current.ControlType -ne [System.Windows.Automation.ControlType]::Edit) {
                    $p = $null
                    if ($i.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$p)) { $p.Select(); break }
                }
            }
            $edits = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::IsPasswordProperty, $true)))
            if ($edits.Count -eq 0) { throw 'no password box in the prompt' }
            $v = $null
            if (-not $edits[0].TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$v)) { throw 'the password box has no value pattern' }
            $v.SetValue($secret)
            $result.button = Use-Button -Window $win -Names @('Yes', 'OK') -Accelerator '{ENTER}'
            $result.answered = $true
        }
    }
}
catch {
    $result.error = $_.Exception.Message
    try { if ($win) { $result.tree = @(Get-UiTree $win) } } catch { Write-Verbose "no tree: $($_.Exception.Message)" }
}
Save-Result -Path $ResultFile
if ($result.error) { exit 1 }
exit 0
