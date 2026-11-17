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
    # The runner is an elevated administrator and the panel refuses to run that way on purpose: this test opts in.
    $env:KORA_ALLOW_ROOT_PANEL = '1'
    $p = Start-Process -FilePath $Kora -ArgumentList @('panel', '--foreground', '--no-open', '--port', "$PanelPort") -PassThru -WindowStyle Hidden -RedirectStandardOutput $out -RedirectStandardError $err
    Remove-Item Env:KORA_ALLOW_ROOT_PANEL -ErrorAction SilentlyContinue
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
        # HttpClient without redirects: PowerShell 7's Invoke-WebRequest throws on a 3xx when redirects are off, and the
        # redirect of the ticket entry (303) is exactly what this checks. One client keeps the gateway cookie.
        $newClient = { $h = New-Object System.Net.Http.HttpClientHandler; $h.AllowAutoRedirect = $false; $h.UseCookies = $true; $c = New-Object System.Net.Http.HttpClient($h); $c.Timeout = [TimeSpan]::FromSeconds(30); $c }
        $status = { param($client, $url) [int]($client.GetAsync($url).GetAwaiter().GetResult().StatusCode) }
        $gateClient = & $newClient
        $enterStatus = & $status $gateClient $t.url
        $viaStatus = & $status $gateClient "http://127.0.0.1:$($gw.Port)/"
        $directStatus = & $status (& $newClient) "http://127.0.0.1:$TxPort/"
        $anonStatus = & $status (& $newClient) "http://127.0.0.1:$($gw.Port)/"
        [pscustomobject]@{ Url = $t.url; Port = $gw.Port; Enter = $enterStatus; Via = $viaStatus; Direct = $directStatus; Anonymous = $anonStatus }
    }
    finally {
        if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
        if ($hadToken) { Set-Content -Path $tokenPath -Value $oldToken -NoNewline } else { Remove-Item $tokenPath -Force -ErrorAction SilentlyContinue }
    }
}

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
