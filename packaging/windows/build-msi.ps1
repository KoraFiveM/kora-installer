<#
.SYNOPSIS
  Build the Kora MSI from a built kora.exe (CI: windows-latest). Needs the WiX .NET tool, which CI installs
  (pinned): dotnet tool install --global wix --version 5.0.2

  packaging/windows/build-msi.ps1 -Exe target\release\kora.exe -Version 0.8.0 -Out dist\kora-windows-x86_64.msi
#>
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$Out,
    # The desktop app (kora-desktop.exe, signed when signing ran). Optional: without it the MSI has no desktop feature.
    [string]$DesktopExe = ''
)
$ErrorActionPreference = 'Stop'
# MSI versions are a.b.c(.d) numbers: drop a leading v and any -rc suffix.
$numeric = ($Version -replace '^v', '') -replace '[-+].*$', ''
if ($numeric -notmatch '^\d+\.\d+\.\d+$') { throw "cannot make an MSI version from '$Version'" }
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$exePath = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Out) | Out-Null
& wix extension add -g WixToolset.Util.wixext/5.0.2
if ($LASTEXITCODE -ne 0) { throw 'wix extension add failed' }
$defines = @('-d', "Version=$numeric", '-d', "KoraExe=$exePath")
if ($DesktopExe) { $defines += @('-d', "DesktopExe=$((Resolve-Path $DesktopExe).Path)") }
& wix build (Join-Path $here 'kora.wxs') -arch x64 -ext WixToolset.Util.wixext @defines -o $Out
if ($LASTEXITCODE -ne 0) { throw 'wix build failed' }
# Look at what was built without installing it: ICE validation, the frozen component GUIDs, the service, the order of the actions.
& (Join-Path $here 'check-msi.ps1') -Msi $Out -WithDesktop:([bool]$DesktopExe)
Write-Host "built $Out ($([math]::Round((Get-Item $Out).Length / 1MB, 1)) MB, version $numeric)"
