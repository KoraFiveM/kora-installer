. "$PSScriptRoot/common.ps1"
$kora = Get-KoraExe
$root = Join-Path $env:RUNNER_TEMP 'kora-ps1'
$rel = Join-Path $root 'release'; $bad = Join-Path $root 'bad'
New-Item -ItemType Directory -Force $rel, $bad | Out-Null
$asset = 'kora-windows-x86_64.exe'
Copy-Item $kora (Join-Path $rel $asset)
$sha = (Get-FileHash -Algorithm SHA256 (Join-Path $rel $asset)).Hash.ToLowerInvariant()
Set-Content -NoNewline -Encoding ascii (Join-Path $rel "$asset.sha256") "$sha  $asset`n"
Copy-Item $kora (Join-Path $bad $asset)
Set-Content -NoNewline -Encoding ascii (Join-Path $bad "$asset.sha256") ("0" * 64 + "  $asset`n")

# Two local release servers (python is on the runner).
$p1 = Start-Process python -ArgumentList '-m', 'http.server', '38081', '--bind', '127.0.0.1', '--directory', $rel -PassThru -WindowStyle Hidden
$p2 = Start-Process python -ArgumentList '-m', 'http.server', '38082', '--bind', '127.0.0.1', '--directory', $bad -PassThru -WindowStyle Hidden
try {
    $ready = $false
    for ($i = 0; $i -lt 30 -and -not $ready; $i++) {
        try { Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:38081/$asset.sha256" | Out-Null; $ready = $true } catch { Start-Sleep -Milliseconds 500 }
    }
    Assert-That $ready 'local fake release server answers'

    $default = Join-Path $env:LOCALAPPDATA 'Kora\bin'
    $userPathBefore = [Environment]::GetEnvironmentVariable('Path', 'User')
    Assert-That (-not (Test-Path "$default\kora.exe")) 'kora.exe is not installed yet'

    # Windows PowerShell 5.1 (what users have), the real `irm | iex` shape: script text through Invoke-Expression.
    $env:KORA_RELEASE_BASE = 'http://127.0.0.1:38081'
    $env:KORA_NO_LAUNCH = '1'
    $out = & powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "iex (Get-Content -Raw '$PWD/scripts/install.ps1')" 2>&1 | Out-String
    $code = $LASTEXITCODE
    Write-Host $out
    Assert-That ($code -eq 0) 'install.ps1 (Windows PowerShell 5.1, via iex) exits 0'
    Assert-That (Test-Path "$default\kora.exe") 'kora.exe installed to %LOCALAPPDATA%\Kora\bin'
    Assert-That ($out -match 'Checksum OK') 'checksum was verified'
    $inst = (Get-FileHash -Algorithm SHA256 "$default\kora.exe").Hash.ToLowerInvariant()
    Assert-That ($inst -eq $sha) 'installed exe has the published sha256'
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    Assert-That (($userPath -split ';') -contains $default) 'the install dir is on the USER path'
    Assert-That (@(($userPath -split ";") | Where-Object { $_ -eq $default }).Count -eq 1) 'and exactly once'
    $v = Invoke-Native "$default\kora.exe" @('--version')
    Assert-That ($v.Code -eq 0) 'the installed kora.exe runs'
    Assert-That ($out -notmatch 'Starting Kora Panel' -and $out -match 'Next step: run   kora panel') 'KORA_NO_LAUNCH=1: the panel is not started, the next step is printed'
    Remove-Item Env:KORA_NO_LAUNCH

    # Second run: up to date, PATH not duplicated, and Kora Panel starts in the background. SSH_CONNECTION
    # makes kora treat this as a computer with no screen, so no browser opens on the runner.
    $env:SSH_CONNECTION = '203.0.113.1 50000 198.51.100.2 22'
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $out = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PWD/scripts/install.ps1" 2>&1 | Out-String
    $code = $LASTEXITCODE
    $sw.Stop()
    Remove-Item Env:SSH_CONNECTION
    Write-Host $out
    Assert-That ($code -eq 0 -and $out -match 'already up to date') 'second run reports up to date'
    Assert-That ($out -match 'Starting Kora Panel') 'the panel is started after installing'
    if ($out -notmatch 'Sign in to Kora Panel') {
        # Diagnostics: why did the background panel not answer?
        Write-Host '== diagnostics: kora processes'
        Get-CimInstance Win32_Process -Filter "Name='kora.exe' or Name='wscript.exe'" | Select-Object ProcessId, CommandLine | Format-List | Out-Host
        Write-Host '== diagnostics: listeners on 4712x'
        netstat -ano | Select-String ':4712' | Out-Host
        Write-Host '== diagnostics: startup script'
        Get-ChildItem "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Startup" -ErrorAction SilentlyContinue | Out-Host
        Get-Content "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Startup\Kora Panel.vbs" -ErrorAction SilentlyContinue | Out-Host
        Write-Host '== diagnostics: kora home'
        Get-ChildItem "$env:LOCALAPPDATA\Kora" -Recurse -File -ErrorAction SilentlyContinue | Select-Object FullName, Length | Out-Host
        Write-Host '== diagnostics: the same command run directly for 10 s'
        $o = Join-Path $env:TEMP 'kora-panel-diag.out'; $e = Join-Path $env:TEMP 'kora-panel-diag.err'
        $dp = Start-Process -FilePath "$default\kora.exe" -ArgumentList 'panel', '--no-open', '--port', '47139' -RedirectStandardOutput $o -RedirectStandardError $e -PassThru -WindowStyle Hidden
        Start-Sleep -Seconds 10
        Write-Host "exited: $($dp.HasExited) code: $(if ($dp.HasExited) { $dp.ExitCode })"
        Get-Content $o, $e -ErrorAction SilentlyContinue | Out-Host
        if (-not $dp.HasExited) { Stop-Process -Id $dp.Id -Force }
    }
    Assert-That ($out -match 'Sign in to Kora Panel' -and $out -match 'http://127\.0\.0\.1:\d+/#(code|token)=[0-9a-f]+') 'the background panel answers and the sign-in link is printed'
    Assert-That ($out -match 'ssh -L \d+:127\.0\.0\.1:\d+ \S+@198\.51\.100\.2') 'with no screen the SSH tunnel command is printed'
    Assert-That ($sw.Elapsed.TotalSeconds -lt 60) 'the launch returns instead of keeping the shell busy'
    $link = Invoke-Native "$default\kora.exe" @('panel', '--print-link')
    Assert-That ($link.Code -eq 0) 'kora panel --print-link works after the install'
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    Assert-That (@(($userPath -split ";") | Where-Object { $_ -eq $default }).Count -eq 1) 'PATH entry is not duplicated by a second run'

    # Checksum mismatch refuses and installs nothing.
    $badDir = Join-Path $root 'bad-target'
    $env:KORA_RELEASE_BASE = 'http://127.0.0.1:38082'; $env:KORA_INSTALL_DIR = $badDir
    $out = & pwsh -NoProfile -File "$PWD/scripts/install.ps1" 2>&1 | Out-String
    Write-Host $out
    Assert-That ($LASTEXITCODE -ne 0) 'checksum mismatch: install.ps1 exits non-zero'
    Assert-That ($out -match 'SHA-256 mismatch') 'checksum mismatch: says why'
    Assert-That (-not (Test-Path "$badDir\kora.exe")) 'checksum mismatch: nothing installed'

    # Non-HTTPS, non-localhost bases refuse (including look-alike hosts).
    foreach ($b in 'http://example.com/releases', 'http://127.0.0.1.evil.example/x', 'http://localhost.evil.example/x', 'ftp://example.com/x') {
        $env:KORA_RELEASE_BASE = $b
        $out = & pwsh -NoProfile -File "$PWD/scripts/install.ps1" 2>&1 | Out-String
        Assert-That ($LASTEXITCODE -ne 0 -and $out -match 'refusing to download') "refuses base $b"
        Assert-That (-not (Test-Path "$badDir\kora.exe")) "nothing installed for $b"
    }

    # Foreign kora.exe is not overwritten without -Force.
    $foreign = Join-Path $root 'foreign'; New-Item -ItemType Directory -Force $foreign | Out-Null
    Set-Content (Join-Path $foreign 'kora.exe') 'not a kora program'
    $env:KORA_RELEASE_BASE = 'http://127.0.0.1:38081'; $env:KORA_INSTALL_DIR = $foreign
    $out = & pwsh -NoProfile -File "$PWD/scripts/install.ps1" 2>&1 | Out-String
    Assert-That ($LASTEXITCODE -ne 0 -and $out -match 'not a kora program') 'a foreign kora.exe is left alone'
    Assert-That ((Get-Content (Join-Path $foreign 'kora.exe') -Raw).Trim() -eq 'not a kora program') 'and untouched'
}
finally {
    if (Test-Path "$env:LOCALAPPDATA\Kora\bin\kora.exe") {
        & "$env:LOCALAPPDATA\Kora\bin\kora.exe" panel --remove-service 2>&1 | Out-Host
    }
    $p1, $p2 | ForEach-Object { if ($_ -and -not $_.HasExited) { Stop-Process -Id $_.Id -Force } }
    Remove-Item Env:KORA_RELEASE_BASE, Env:KORA_INSTALL_DIR, Env:KORA_NO_LAUNCH, Env:SSH_CONNECTION -ErrorAction SilentlyContinue
}

exit 0
