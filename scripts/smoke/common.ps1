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

# txAdmin through the panel's own address (the txAdmin gateway, crates/kora-web/src/txgate): start the panel in this
# session with a known sign-in token, sign in, ask for a one-time ticket for the server, enter with it, and fetch
# txAdmin through the panel's port for the server (txAdmin's port + 10000). Returns what was seen; the caller asserts.
# The panel is stopped again before returning. Run only while the server's txAdmin answers on 127.0.0.1:$TxPort.
function Test-TxAdminThroughPanel([string]$Kora, [string]$ServerName, [int]$TxPort, [int]$PanelPort = 47131) {
    $koraHome = Join-Path $env:LOCALAPPDATA 'Kora'
    $tokenPath = Join-Path $koraHome 'panel-token'
    $hadToken = Test-Path $tokenPath
    if ($hadToken) { $oldToken = Get-Content $tokenPath -Raw }
    New-Item -ItemType Directory -Force -Path $koraHome | Out-Null
    $token = -join ((1..32) | ForEach-Object { '{0:x2}' -f (Get-Random -Maximum 256) })
    Set-Content -Path $tokenPath -Value $token -NoNewline
    $out = Join-Path $env:TEMP 'kora-panel-smoke.out'; $err = Join-Path $env:TEMP 'kora-panel-smoke.err'
    $p = Start-Process -FilePath $Kora -ArgumentList @('panel', '--foreground', '--no-open', '--port', "$PanelPort") -PassThru -WindowStyle Hidden -RedirectStandardOutput $out -RedirectStandardError $err
    try {
        $up = $false
        $end = (Get-Date).AddSeconds(40)
        while ((Get-Date) -lt $end -and -not $up) {
            try { $null = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$PanelPort/api/v1/meta" -TimeoutSec 3; $up = $true } catch { Start-Sleep -Seconds 1 }
        }
        if (-not $up) { Get-Content $out, $err -ErrorAction SilentlyContinue | Write-Host; throw 'the panel did not start for the txAdmin gateway check' }
        $base = "http://127.0.0.1:$PanelPort/api/v1"
        $headers = @{ 'X-Kora' = '1' }
        $s = Invoke-RestMethod -Method Post -Uri "$base/session" -Headers $headers -ContentType 'application/json' -Body (@{ token = $token } | ConvertTo-Json) -SessionVariable panelWeb
        $headers['X-Kora-Session'] = $s.secret
        $t = Invoke-RestMethod -Method Post -Uri "$base/servers/$ServerName/txadmin/ticket" -Headers $headers -WebSession $panelWeb
        $gw = [uri]$t.url
        $gate = New-Object Microsoft.PowerShell.Commands.WebRequestSession
        $enter = Invoke-WebRequest -UseBasicParsing -Uri $t.url -WebSession $gate -MaximumRedirection 0 -SkipHttpErrorCheck -TimeoutSec 15
        $via = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$($gw.Port)/" -WebSession $gate -MaximumRedirection 0 -SkipHttpErrorCheck -TimeoutSec 20
        $direct = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$TxPort/" -MaximumRedirection 0 -SkipHttpErrorCheck -TimeoutSec 20
        $anon = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$($gw.Port)/" -MaximumRedirection 0 -SkipHttpErrorCheck -TimeoutSec 15
        [pscustomobject]@{ Url = $t.url; Port = $gw.Port; Enter = [int]$enter.StatusCode; Via = [int]$via.StatusCode; Direct = [int]$direct.StatusCode; Anonymous = [int]$anon.StatusCode }
    }
    finally {
        if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
        if ($hadToken) { Set-Content -Path $tokenPath -Value $oldToken -NoNewline } else { Remove-Item $tokenPath -Force -ErrorAction SilentlyContinue }
    }
}
