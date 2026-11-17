# Smoke test of the Kora desktop app on Windows (the runner has a desktop session and WebView2).
# Usage: desktop-smoke.ps1 -Dist <folder with kora.exe and kora-desktop.exe>
# See desktop-smoke.sh: the same steps. The runner is an elevated administrator, so KORA_ALLOW_ROOT_PANEL=1 (set by the
# workflow) lets the panel start; real people run it as a normal user.
param([Parameter(Mandatory = $true)][string]$Dist)
$ErrorActionPreference = 'Stop'
$Dist = (Resolve-Path $Dist).Path
$work = Join-Path $env:RUNNER_TEMP ("desktop-smoke-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force "$work\kora-home", "$work\desktop" | Out-Null
$env:KORA_HOME = "$work\kora-home"
$env:KORA_DESKTOP_HOME = "$work\desktop"
$env:KORA_DESKTOP_SMOKE_WAIT_SECOND = '1'
$report = "$work\desktop\report.json"

$run = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$before = (Get-ItemProperty -Path $run -Name Kora -ErrorAction SilentlyContinue).Kora

$p = Start-Process -FilePath "$Dist\kora-desktop.exe" -ArgumentList @('--kora', "$Dist\kora.exe", '--smoke', $report) -PassThru
$ready = $false
for ($i = 0; $i -lt 120; $i++) {
  if (Test-Path "$report.ready") { $ready = $true; break }
  if ($p.HasExited) { break }
  Start-Sleep -Seconds 1
}
if (-not $ready) {
  Write-Host "::error::the desktop app never showed the panel"
  if (Test-Path $report) { Get-Content $report }
  if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
  exit 1
}

# A second launch asks the first to show its window and exits at once, successfully.
$second = Start-Process -FilePath "$Dist\kora-desktop.exe" -ArgumentList @('--open', '/system') -PassThru -Wait
if ($second.ExitCode -ne 0) { Write-Host "::error::the second launch exited with $($second.ExitCode)"; exit 1 }

if (-not $p.WaitForExit(120000)) { Write-Host "::error::the desktop app did not finish"; Stop-Process -Id $p.Id -Force; exit 1 }
if ($p.ExitCode -ne 0) { Write-Host "::error::the desktop app exited with $($p.ExitCode)"; if (Test-Path $report) { Get-Content $report }; exit 1 }
Get-Content $report
$r = Get-Content $report -Raw | ConvertFrom-Json

function Check($ok, $what) { if (-not $ok) { Write-Host "::error::report check failed: $what"; exit 1 } }
Check ($r.ok -eq $true) 'ok'
Check ($r.window_title -eq 'Kora') 'window title is Kora'
Check ($r.page_title -match 'Kora') 'page title mentions Kora'
Check ($r.url_before_action.StartsWith('http://127.0.0.1:')) 'the window shows the panel on loopback'
Check ($r.action_url -like '*/settings*') 'the tray action opened the settings page'
Check ($r.connected -eq $true) 'the event stream is connected'
Check ($r.tray_built -eq $true) 'the notification-area icon was made'
Check ($r.menu_ids -contains 'quit') 'menu has Quit'
Check ($r.menu_ids -contains 'quit-stop') 'menu has Quit and turn off the background service'
Check ($r.menu_ids -contains 'open:/') 'menu has Open Kora'
Check ($r.second_launch -eq '/system') 'the second launch reached the first'
# Smoke mode must not change start-up.
$after = (Get-ItemProperty -Path $run -Name Kora -ErrorAction SilentlyContinue).Kora
Check ($before -eq $after) 'start-up entry unchanged'
Get-Process -Name kora -ErrorAction SilentlyContinue | Stop-Process -Force
Write-Host 'desktop smoke test passed'
