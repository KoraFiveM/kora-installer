<#
.SYNOPSIS
  Installs the Kora program for the current user (no administrator rights needed).

.DESCRIPTION
  Downloads kora.exe from the newest stable release, checks its SHA-256, copies it to
  %LOCALAPPDATA%\Kora\bin and adds that folder to your user PATH. An existing kora.exe is only
  replaced when it really is an earlier kora (the old one is kept as kora.exe.previous).

  Then it starts Kora Panel in the background (it also starts at every sign-in) with
  `kora panel --setup`, opens it in your browser and prints the sign-in link. If that step fails
  the install still counts as done and the manual next step is printed.

  Settings (parameters or environment variables, all optional):
    -Base / KORA_RELEASE_BASE   Where the release files live. https:// only (http:// for localhost).
    -InstallDir / KORA_INSTALL_DIR   Target folder. Default: %LOCALAPPDATA%\Kora\bin
    -Version / KORA_VERSION     Release to install, e.g. v0.2.0 (any release, pre-releases too).
                                Default: the newest stable release (not a pre-release, not a draft).
    -Prerelease / KORA_PRERELEASE=1   Without -Version: take the newest release even if it is a pre-release.
    -Force                      Replace an existing kora.exe even if it is not a kora program.
    -NoLaunch / KORA_NO_LAUNCH=1   Only install the program: do not start Kora Panel or open a browser.

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
    [switch]$NoLaunch,
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
    $response = Invoke-WebRequest @request
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

    $dest = Join-Path $InstallDir 'kora.exe'
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
finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ''
if ($NoLaunch -or $env:KORA_NO_LAUNCH -eq '1') {
    Write-Host 'Next step: run   kora panel   (opens Kora Panel in your browser)'
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
Write-Host 'Prefer the terminal? Run   kora install   instead.'
