<#
.SYNOPSIS
  Installs Kora: for the whole computer (recommended, one administrator approval) or just for you.

.DESCRIPTION
  First choice, the system install: downloads the Kora installer (.msi) from the newest stable release,
  checks its SHA-256 and runs it with ONE Windows permission prompt. Kora goes into Program Files together
  with Kora's small administrator service, so installing servers, firewall rules and databases later needs
  no prompt at all. It asks before doing this ("Install Kora for this whole computer?"); answer n, or press
  No at the Windows prompt, to install just for you instead.

  The per-user install (no administrator rights): downloads kora.exe from the same release, checks its
  SHA-256, copies it to %LOCALAPPDATA%\Kora\bin and adds that folder to your user PATH. The parts of a
  server that need administrator rights are then "continue without" or a command to run. An existing
  kora.exe is only replaced when it really is an earlier kora (the old one is kept as kora.exe.previous).

  It also makes a "Kora" shortcut in your Start Menu and on your Desktop (yours only, no administrator
  rights). They open Kora Panel in your browser.

  Then it starts Kora Panel in the background (it also starts at every sign-in) with
  `kora panel --setup`, opens it in your browser and prints the sign-in link. If that step fails
  the install still counts as done and the manual next step is printed.

  Settings (parameters or environment variables, all optional):
    -Base / KORA_RELEASE_BASE   Where the release files live. https:// only (http:// for localhost).
    -System / KORA_SYSTEM=1     Do the system install without asking (fail if it cannot be done).
    -PerUser / KORA_SYSTEM=0    Install just for you, without asking and without administrator rights.
    -InstallDir / KORA_INSTALL_DIR   Target folder (a per-user install: the system install is not offered). Default: %LOCALAPPDATA%\Kora\bin
    -Version / KORA_VERSION     Release to install, e.g. v0.2.0 (any release, pre-releases too).
                                Default: the newest stable release (not a pre-release, not a draft).
    -Prerelease / KORA_PRERELEASE=1   Without -Version: take the newest release even if it is a pre-release.
    -Force                      Replace an existing kora.exe even if it is not a kora program.
    -NoLaunch / KORA_NO_LAUNCH=1   Only install the program: do not start Kora Panel or open a browser.
    -NoShortcuts / KORA_NO_SHORTCUTS=1   Do not make the Start Menu and Desktop shortcuts.

  Downloads use https only: a download that was redirected to plain http is refused.

.EXAMPLE
  irm <release-base>/install.ps1 | iex
#>
[CmdletBinding()]
param(
    [string]$Base = $(if ($env:KORA_RELEASE_BASE) { $env:KORA_RELEASE_BASE } else { 'https://github.com/KoraFiveM/kora-installer/releases/latest/download' }),
    [string]$InstallDir = $(if ($env:KORA_INSTALL_DIR) { $env:KORA_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Kora\bin' }),
    [string]$Version = $env:KORA_VERSION,
    [switch]$Force,
    [switch]$System,
    [switch]$PerUser,
    [switch]$NoLaunch,
    [switch]$NoShortcuts,
    [switch]$Prerelease
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Exit-Install([string]$Message) {
    Write-Error "kora install: $Message" -ErrorAction Stop
}

# The download address must be https (plain http only for this machine).
$Base = $Base.TrimEnd('/')
if ($Base -notmatch '^https://' -and $Base -notmatch '^http://(localhost|127\.0\.0\.1)(:\d+)?(/|$)') {
    Exit-Install "refusing to download from '$Base': the release address must start with https:// (plain http:// is only allowed for localhost)."
}

if ($env:PROCESSOR_ARCHITECTURE -notin @('AMD64', 'ARM64')) {
    Exit-Install "there is no kora build for the '$($env:PROCESSOR_ARCHITECTURE)' processor (only 64-bit Windows)."
}

# Fetch $Uri (into $OutFile when given) and make sure https stayed https: Invoke-WebRequest follows
# redirects by itself (Windows PowerShell 5.1 even from https to http), so the address the answer
# finally came from is checked, and the download is thrown away if it is not https.
function Invoke-HttpsOnly([string]$Uri, [string]$OutFile) {
    $request = @{ UseBasicParsing = $true; Uri = $Uri }
    if ($OutFile) { $request.OutFile = $OutFile; $request.PassThru = $true }
    # Windows PowerShell 5.1 redraws a progress bar for every chunk, which makes a 30 MB download take over a minute.
    $previousProgress = $ProgressPreference
    $ProgressPreference = 'SilentlyContinue'
    try { $response = Invoke-WebRequest @request }
    finally { $ProgressPreference = $previousProgress }
    if ($Uri -match '^https://') {
        $final = $null
        $raw = $response.BaseResponse
        if ($raw.PSObject.Properties['ResponseUri']) { $final = $raw.ResponseUri }                       # Windows PowerShell 5.1
        elseif ($raw.PSObject.Properties['RequestMessage']) { $final = $raw.RequestMessage.RequestUri }  # PowerShell 7
        if (-not $final -or $final.Scheme -ne 'https') {
            if ($OutFile) { Remove-Item -LiteralPath $OutFile -Force -ErrorAction SilentlyContinue }
            Exit-Install "refusing the download from $Uri`: it did not stay on https (ended at '$final')."
        }
    }
    $response
}

function Get-Text($Response) {
    $text = $Response.Content
    if ($text -is [byte[]]) { $text = [Text.Encoding]::UTF8.GetString($text) }
    $text
}

# The system install: the checked .msi with ONE permission prompt. $true when it is installed.
function Install-System([string]$Source, [string]$Work) {
    $msiAsset = 'kora-windows-x86_64.msi'
    $msi = Join-Path $Work 'kora.msi'
    Invoke-HttpsOnly -Uri "$Source/$msiAsset" -OutFile $msi | Out-Null
    $want = ((Get-Text -Response (Invoke-HttpsOnly -Uri "$Source/$msiAsset.sha256")).Trim() -split '\s+')[0].ToLowerInvariant()
    if ($want -notmatch '^[0-9a-f]{64}$') { Write-Host 'The installer checksum file is not a SHA-256 value.'; return $false }
    # Lock the file FIRST (nobody can write or replace it from now on), then check the very bytes msiexec will read.
    $lock = [IO.File]::Open($msi, 'Open', 'Read', 'Read')
    try {
        $got = (Get-FileHash -Algorithm SHA256 -InputStream $lock).Hash.ToLowerInvariant()
        if ($got -ne $want) { Write-Host 'The installer does not match its checksum (SHA-256). Nothing was installed.'; return $false }
        Write-Host 'Installer checksum OK. Windows asks for administrator permission once: choose Yes.'
        $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        $arguments = "/i `"$msi`" /passive /norestart ALLOWUSERS=`"$sid`""
        try {
            $p = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\msiexec.exe') -ArgumentList $arguments -Verb RunAs -Wait -PassThru
        }
        catch { Write-Host "The installer did not run: $($_.Exception.Message)"; return $false }
        if ($p.ExitCode -ne 0 -and $p.ExitCode -ne 3010) { Write-Host "The installer stopped with code $($p.ExitCode)."; return $false }
    }
    finally { $lock.Dispose() }
    Test-Path -LiteralPath (Join-Path $env:ProgramFiles 'Kora\kora.exe')
}

# Older Windows PowerShell does not use TLS 1.2 by default.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$asset = 'kora-windows-x86_64.exe'

# Which release: -Version / KORA_VERSION picks one explicitly (pre-releases too). Otherwise the newest
# stable release (not a draft, not a pre-release) from /releases?per_page=30, or with -Prerelease /
# KORA_PRERELEASE=1 the newest release of any kind. If the lookup fails we fall back to GitHub's
# "latest", which is also the newest stable release. Skipped entirely when a base address is given.
if (-not $env:KORA_RELEASE_BASE -and -not $PSBoundParameters.ContainsKey('Base')) {
    $tag = $null
    if ($Version) {
        $tag = if ($Version.StartsWith('v')) { $Version } else { "v$Version" }
        if ($tag -notmatch '^[A-Za-z0-9._-]+$') { Exit-Install "'$Version' is not a valid release name." }
    }
    else {
        $anyRelease = $Prerelease -or $env:KORA_PRERELEASE -eq '1'
        try {
            $list = Get-Text -Response (Invoke-HttpsOnly -Uri 'https://api.github.com/repos/KoraFiveM/kora-installer/releases?per_page=30') | ConvertFrom-Json
            # foreach, not the pipeline: Windows PowerShell 5.1 hands ConvertFrom-Json's array over as one object.
            foreach ($rel in $list) {
                if (-not $rel.draft -and ($anyRelease -or -not $rel.prerelease)) {
                    if ($rel.tag_name -match '^[A-Za-z0-9._-]+$') { $tag = $rel.tag_name }
                    break
                }
            }
        } catch { Write-Verbose "could not look up the newest release: $($_.Exception.Message)" }
    }
    if ($tag) { $Base = "https://github.com/KoraFiveM/kora-installer/releases/download/$tag" }
}
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("kora-install-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    Write-Host "Downloading $asset from $Base ..."
    $exe = Join-Path $tmp 'kora.exe'
    Invoke-HttpsOnly -Uri "$Base/$asset" -OutFile $exe | Out-Null
    $sumText = Get-Text -Response (Invoke-HttpsOnly -Uri "$Base/$asset.sha256")

    $expected = ($sumText.Trim() -split '\s+')[0].ToLowerInvariant()
    if ($expected -notmatch '^[0-9a-f]{64}$') { Exit-Install 'the checksum file is not a SHA-256 value.' }
    $actual = (Get-FileHash -Algorithm SHA256 -Path $exe).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { Exit-Install 'the download is corrupt or has been tampered with (SHA-256 mismatch). Nothing was installed.' }
    Write-Host "Checksum OK ($actual)."

    # The system install first (see the description); anything that goes wrong falls back to the per-user
    # install below unless it was asked for with -System / KORA_SYSTEM=1.
    $wantSystem = $false
    $mustSystem = $System -or $env:KORA_SYSTEM -eq '1'
    if ($mustSystem) { $wantSystem = $true }
    elseif (-not ($PerUser -or $env:KORA_SYSTEM -eq '0' -or $PSBoundParameters.ContainsKey('InstallDir') -or $env:KORA_INSTALL_DIR)) {
        if ([Environment]::UserInteractive -and -not [Console]::IsInputRedirected) {
            $answer = Read-Host 'Install Kora for this whole computer (recommended)? It asks for administrator permission once; after that Kora can set up servers, firewall rules and databases by itself. Answer n to install just for you [Y/n]'
            $wantSystem = $answer -eq '' -or $answer -match '^(y|yes)$'
        }
    }
    $systemDone = $false
    if ($wantSystem) {
        $systemDone = Install-System -Source $Base -Work $tmp
        if ($systemDone) { Write-Host 'Kora is installed for this computer, with its administrator service.' }
        elseif ($mustSystem) { Exit-Install 'the system install did not work (-System / KORA_SYSTEM=1, so nothing else was tried). Run it again, or use -PerUser to install just for you.' }
        else { Write-Host 'The system install was not done; installing just for you instead (no administrator rights needed).' }
    }
    $dest = if ($systemDone) { Join-Path $env:ProgramFiles 'Kora\kora.exe' } else { Join-Path $InstallDir 'kora.exe' }
    if (-not $systemDone) {
        $upToDate = $false
        if (Test-Path -LiteralPath $dest) {
            $current = (Get-FileHash -Algorithm SHA256 -Path $dest).Hash.ToLowerInvariant()
            if ($current -eq $actual) {
                Write-Host "kora is already up to date at $dest."
                $upToDate = $true
            }
            elseif ($Force -or (Select-String -Path $dest -Pattern 'Kora installer and server manager' -Quiet -Encoding ascii)) {
                Move-Item -LiteralPath $dest -Destination "$dest.previous" -Force
                Write-Host "The earlier version was kept as $dest.previous."
            }
            else {
                Exit-Install "$dest already exists and is not a kora program, so it was left alone. Use -InstallDir for another folder or -Force to replace it."
            }
        }
        if (-not $upToDate) {
            New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
            Copy-Item -LiteralPath $exe -Destination $dest -Force
            Write-Host "Installed kora to $dest"
        }

        # Put the folder on the user's PATH (current user only, no admin).
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $entries = @($userPath -split ';' | Where-Object { $_ })
        if ($entries -notcontains $InstallDir) {
            [Environment]::SetEnvironmentVariable('Path', (($entries + $InstallDir) -join ';'), 'User')
            $env:Path = "$env:Path;$InstallDir"
            Write-Host "Added $InstallDir to your PATH (new terminals pick it up)."
        }
    }
    }
finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

# The shortcuts that open the panel (per user, no admin). A failure here does not undo the install.
if (-not ($NoShortcuts -or $env:KORA_NO_SHORTCUTS -eq '1')) {
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $dest app install --yes
        if ($LASTEXITCODE -ne 0) { Write-Host 'The Start Menu shortcut could not be made. You can still open Kora by running   kora' }
    }
    catch { Write-Host "Could not make the shortcuts: $($_.Exception.Message)" }
    finally { $ErrorActionPreference = $previousPreference }
}

Write-Host ''
if ($NoLaunch -or $env:KORA_NO_LAUNCH -eq '1') {
    Write-Host 'Next step: run   kora   (opens Kora Panel in your browser)'
}
else {
    Write-Host 'Starting Kora Panel ...'
    # The panel runs in the background, so this returns within a few seconds. A failure here does not
    # undo the install. 'Continue' so a message on stderr is shown, not turned into a stopping error.
    $launched = $false
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $dest panel --setup
        $launched = ($LASTEXITCODE -eq 0)
    }
    catch { Write-Host "Could not run kora: $($_.Exception.Message)" }
    finally { $ErrorActionPreference = $previousPreference }
    Write-Host ''
    if (-not $launched) {
        Write-Host 'Kora Panel did not start by itself. kora is installed; start the panel with:'
        Write-Host "    & '$dest' panel"
        $global:LASTEXITCODE = 0  # the install itself worked
    }
}
Write-Host 'Open Kora again any time from the Start Menu or the Desktop shortcut, or run   kora   in a terminal.'
