# `kora self-update` against a local fake GitHub (python http.server + KORA_SELF_UPDATE_API, loopback only):
# copy A replaces ITSELF with copy B while running (Windows renames the running exe aside), a bad checksum
# changes nothing, a release without the asset is refused, offline is a clear error, `.old` goes at next start.
. "$PSScriptRoot/common.ps1"
$kora = Get-KoraExe
$root = Join-Path $env:RUNNER_TEMP 'kora-selfupd'
$run = Join-Path $root 'run'; $srv = Join-Path $root 'srv'; $dl = Join-Path $srv 'dl'
$api = Join-Path $srv 'repos\KoraFiveM\kora-installer'
Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $run, $dl, $api, (Join-Path $root 'home') | Out-Null
$asset = 'kora-windows-x86_64.exe'
$a = Join-Path $run 'kora.exe'
Copy-Item $kora $a
# B = A plus a trailing overlay: still a valid exe, different bytes (so a different hash).
Copy-Item $kora (Join-Path $dl $asset)
$fs = [IO.File]::Open((Join-Path $dl $asset), 'Append'); $pad = [Text.Encoding]::ASCII.GetBytes('SELFUPDATE-B'); $fs.Write($pad, 0, $pad.Length); $fs.Close()
$hashA = (Get-FileHash -Algorithm SHA256 $a).Hash.ToLowerInvariant()
$hashB = (Get-FileHash -Algorithm SHA256 (Join-Path $dl $asset)).Hash.ToLowerInvariant()
Assert-That ($hashA -ne $hashB) 'copies A and B differ'

$port = 38090
$base = "http://127.0.0.1:$port"
function Set-Sum([string]$h) { Set-Content -NoNewline -Encoding ascii (Join-Path $dl "$asset.sha256") "$h  $asset`n" }
function Set-Releases([bool]$withAsset) {
    $assets = @()
    if ($withAsset) {
        $assets = @(
            @{ name = $asset; browser_download_url = "$base/dl/$asset"; size = 1 },
            @{ name = "$asset.sha256"; browser_download_url = "$base/dl/$asset.sha256"; size = 1 })
    }
    $json = ConvertTo-Json -Depth 5 -InputObject @(@{ tag_name = 'v99.0.0'; draft = $false; prerelease = $false; assets = $assets })
    Set-Content -Encoding ascii (Join-Path $api 'releases') $json
}
$envs = @{ KORA_SELF_UPDATE_API = $base; KORA_HOME = (Join-Path $root 'home'); KORA_NO_UPDATE_CHECK = '1' }

$p = Start-Process python -ArgumentList '-m', 'http.server', "$port", '--bind', '127.0.0.1', '--directory', $srv -PassThru -WindowStyle Hidden
try {
    Set-Releases $true; Set-Sum ('0' * 64)
    $ready = $false
    for ($i = 0; $i -lt 30 -and -not $ready; $i++) {
        try { Invoke-WebRequest -UseBasicParsing "$base/repos/KoraFiveM/kora-installer/releases" | Out-Null; $ready = $true } catch { Start-Sleep -Milliseconds 500 }
    }
    Assert-That $ready 'local fake release API answers'

    $r = Invoke-Native $a @('self-update', '--check') $envs; Show 'self-update --check' $r
    Assert-That ($r.Code -eq 0 -and $r.Output -match 'v99\.0\.0 is available') '--check reports the newer release'
    Assert-That ((Get-FileHash -Algorithm SHA256 $a).Hash.ToLowerInvariant() -eq $hashA) '--check changes nothing'

    # checksum mismatch: nothing changes
    $r = Invoke-Native $a @('self-update', '--yes') $envs; Show 'self-update (bad checksum)' $r
    Assert-That ($r.Code -ne 0 -and $r.Output -match 'does not match its checksum') 'a checksum mismatch is refused'
    Assert-That ((Get-FileHash -Algorithm SHA256 $a).Hash.ToLowerInvariant() -eq $hashA) 'the old exe is intact'
    Assert-That (-not (Test-Path "$a.old")) 'no .old was made'
    Assert-That (@(Get-ChildItem $run -Force).Count -eq 1) 'no temp file is left behind'

    # no asset in the release: refused
    Set-Releases $false
    $r = Invoke-Native $a @('self-update', '--yes') $envs; Show 'self-update (no asset)' $r
    Assert-That ($r.Code -ne 0 -and $r.Output -match 'has no kora-windows-x86_64.exe') 'a release without our file is refused'
    Assert-That ((Get-FileHash -Algorithm SHA256 $a).Hash.ToLowerInvariant() -eq $hashA) 'still intact'

    # offline
    $r = Invoke-Native $a @('self-update', '--yes') @{ KORA_SELF_UPDATE_API = 'http://127.0.0.1:1'; KORA_HOME = (Join-Path $root 'home'); KORA_NO_UPDATE_CHECK = '1' }; Show 'self-update (offline)' $r
    Assert-That ($r.Code -ne 0 -and $r.Output -match 'could not reach GitHub') 'offline gives a clear error'

    # the real thing: A (running) replaces itself with B
    Set-Releases $true; Set-Sum $hashB
    $r = Invoke-Native $a @('self-update', '--yes') $envs; Show 'self-update A -> B' $r
    Assert-That ($r.Code -eq 0 -and $r.Output -match 'Updated') 'self-update exits 0'
    Assert-That ((Get-FileHash -Algorithm SHA256 $a).Hash.ToLowerInvariant() -eq $hashB) 'B is in place at the original path'
    Assert-That (Test-Path "$a.old") 'the previous exe was renamed to kora.exe.old'
    Assert-That ((Get-FileHash -Algorithm SHA256 "$a.old").Hash.ToLowerInvariant() -eq $hashA) '.old is the previous program'

    # next start cleans .old
    $r = Invoke-Native $a @('--version') $envs; Show 'kora --version' $r
    Assert-That ($r.Code -eq 0) 'the new exe runs'
    Assert-That (-not (Test-Path "$a.old")) '.old is deleted at the next start'
    Assert-That (@(Get-ChildItem $run -Force).Count -eq 1) 'only kora.exe remains'

    # B is not newer than... (B reports the same version as A): update offered again, never a downgrade path without --version
    $r = Invoke-Native $a @('self-update', '--check') $envs
    Assert-That ($r.Output -match 'v99\.0\.0 is available') 'the notice path still works from B'

    # ---- the installed Kora app copy (Program Files) is what gets updated, not the running copy ----
    $pfDir = Join-Path $env:ProgramFiles 'Kora'; $pf = Join-Path $pfDir 'kora.exe'
    $r = Invoke-Native $kora @('app', 'install', '--yes') $envs; Show 'app install (copy A)' $r
    Assert-That ($r.Code -eq 0 -and (Test-Path $pf)) 'the app copy is installed in Program Files'
    Assert-That ((Get-FileHash -Algorithm SHA256 $pf).Hash.ToLowerInvariant() -eq $hashA) 'the app copy is build A'
    $runBefore = (Get-FileHash -Algorithm SHA256 $a).Hash.ToLowerInvariant()
    # checksum mismatch first: the app copy stays
    Set-Sum ('1' * 64)
    $r = Invoke-Native $a @('self-update', '--yes') $envs; Show 'self-update (bad checksum, app copy)' $r
    Assert-That ($r.Code -ne 0) 'mismatch refused'
    Assert-That ((Get-FileHash -Algorithm SHA256 $pf).Hash.ToLowerInvariant() -eq $hashA) 'the app copy is intact'
    Set-Sum $hashB
    $r = Invoke-Native $a @('self-update', '--yes') $envs; Show 'self-update (app copy)' $r
    Assert-That ($r.Code -eq 0 -and $r.Output -match [regex]::Escape($pf)) 'self-update targets Program Files\Kora\kora.exe'
    Assert-That ((Get-FileHash -Algorithm SHA256 $pf).Hash.ToLowerInvariant() -eq $hashB) 'the app copy is now build B'
    Assert-That ((Get-FileHash -Algorithm SHA256 $a).Hash.ToLowerInvariant() -eq $runBefore) 'the running copy elsewhere was not touched'
    Assert-That (Test-Path "$pf.old") 'the previous app exe was renamed to kora.exe.old'
    $r = Invoke-Native $pf @('app', 'status') $envs; Show 'app status after self-update' $r
    Assert-That ($r.Output -match 'version: 99\.0\.0') 'app status reports the new recorded version'
    Assert-That (-not (Test-Path "$pf.old")) '.old of the app copy is deleted at the next start'
    $r = Invoke-Native $kora @('app', 'uninstall', '--yes') $envs; Show 'app uninstall' $r
    Assert-That (-not (Test-Path $pfDir)) 'cleanup: Program Files\Kora is gone'
}
finally {
    $d = Join-Path $env:ProgramFiles 'Kora'
    if (Test-Path $d) { & $kora app uninstall --yes 2>&1 | Out-Null }
    if ($p) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
}
