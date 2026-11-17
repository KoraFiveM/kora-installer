# Shared helpers for the Windows smoke job. Dot-source: . scripts/smoke/common.ps1
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-That([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "ASSERTION FAILED: $Message" }
    Write-Host "ok: $Message"
}

# Run a native command, capture stdout+stderr as text and the exit code (never throws on non-zero).
function Invoke-Native([string]$File, [string[]]$Arguments, [hashtable]$Env = @{}) {
    $old = @{}
    foreach ($k in $Env.Keys) { $old[$k] = [Environment]::GetEnvironmentVariable($k); [Environment]::SetEnvironmentVariable($k, $Env[$k]) }
    $prev = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try {
        $out = & $File @Arguments 2>&1 | ForEach-Object { "$_" } | Out-String
        $code = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $prev
        foreach ($k in $old.Keys) { [Environment]::SetEnvironmentVariable($k, $old[$k]) }
    }
    [pscustomobject]@{ Output = $out; Code = $code }
}

function Show([string]$Title, $Result) {
    Write-Host "--- $Title (exit $($Result.Code)) ---"
    Write-Host $Result.Output
}

function Get-KoraExe { (Resolve-Path "$PSScriptRoot/../../target/release/kora.exe").Path }

# The ACL of a file as text, e.g. for "is Users able to read this?".
function Get-Acl-Text([string]$Path) { (& icacls $Path | Out-String) }

# The server list lives in Kora's data file now (<Kora folder>\store\kora.db), not in servers.json. `kora store export-json`
# writes it out in the old servers.json format: this returns that text ('' when no server is registered).
function Get-RegistryText([hashtable]$Env = @{}) {
  $out = Join-Path ([IO.Path]::GetTempPath()) ("kora-reg-" + [guid]::NewGuid().ToString('N'))
  try {
    [void](Invoke-Native (Get-KoraExe) @('store', 'export-json', '--to', $out) $Env)
    $f = Join-Path $out 'servers.json'
    if (Test-Path $f) { return (Get-Content $f -Raw) }
    return ''
  } finally { if (Test-Path $out) { Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue } }
}
