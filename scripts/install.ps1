<#
.SYNOPSIS
  Installs the Kora program for the current user (no administrator rights needed).

.DESCRIPTION
  Downloads kora.exe from the newest published release, checks its SHA-256, copies it to
  %LOCALAPPDATA%\Kora\bin and adds that folder to your user PATH. An existing kora.exe is only
  replaced when it really is an earlier kora (the old one is kept as kora.exe.previous).

  Settings (parameters or environment variables, all optional):
    -Base / KORA_RELEASE_BASE   Where the release files live. https:// only (http:// for localhost).
    -InstallDir / KORA_INSTALL_DIR   Target folder. Default: %LOCALAPPDATA%\Kora\bin
    -Version / KORA_VERSION     Release to install, e.g. v0.2.0. Default: newest published release (pre-releases included).
    -Force                      Replace an existing kora.exe even if it is not a kora program.

.EXAMPLE
  irm <release-base>/install.ps1 | iex
#>
[CmdletBinding()]
param(
    [string]$Base = $(if ($env:KORA_RELEASE_BASE) { $env:KORA_RELEASE_BASE } else { 'https://github.com/KoraFiveM/kora-installer/releases/latest/download' }),
    [string]$InstallDir = $(if ($env:KORA_INSTALL_DIR) { $env:KORA_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Kora\bin' }),
    [string]$Version = $env:KORA_VERSION,
    [switch]$Force
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

# Older Windows PowerShell does not use TLS 1.2 by default.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$asset = 'kora-windows-x86_64.exe'

# GitHub's "latest" never points at a pre-release, so by default ask for the newest published release
# (pre-releases included) from /releases?per_page=10; -Version / KORA_VERSION picks one explicitly.
# If the lookup fails we fall back to "latest". Skipped entirely when a base address is given.
if (-not $env:KORA_RELEASE_BASE -and -not $PSBoundParameters.ContainsKey('Base')) {
    $tag = $null
    if ($Version) {
        $tag = if ($Version.StartsWith('v')) { $Version } else { "v$Version" }
        if ($tag -notmatch '^[A-Za-z0-9._-]+$') { Exit-Install "'$Version' is not a valid release name." }
    }
    else {
        try {
            $list = @(Invoke-RestMethod -UseBasicParsing -Uri 'https://api.github.com/repos/KoraFiveM/kora-installer/releases?per_page=10')
            $rel = $list | Where-Object { -not $_.draft } | Select-Object -First 1
            if ($rel -and $rel.tag_name -match '^[A-Za-z0-9._-]+$') { $tag = $rel.tag_name }
        } catch { Write-Verbose "could not look up the newest release: $($_.Exception.Message)" }
    }
    if ($tag) { $Base = "https://github.com/KoraFiveM/kora-installer/releases/download/$tag" }
}
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("kora-install-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    Write-Host "Downloading $asset from $Base ..."
    $exe = Join-Path $tmp 'kora.exe'
    Invoke-WebRequest -UseBasicParsing -Uri "$Base/$asset" -OutFile $exe
    $sumText = (Invoke-WebRequest -UseBasicParsing -Uri "$Base/$asset.sha256").Content
    if ($sumText -is [byte[]]) { $sumText = [Text.Encoding]::ASCII.GetString($sumText) }

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
Write-Host 'Next step: run   kora install'
Write-Host '(Add --service later if the server should start with Windows; that part needs an administrator terminal.)'
