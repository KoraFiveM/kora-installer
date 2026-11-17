# TEMPORARY probe (branch nx/ci-win): what does `kora service allow` do under the installer, and where does it wait?
param([Parameter(Mandatory)][string]$Msi)
$ErrorActionPreference = 'Continue'
$kora = (Resolve-Path 'target/release/kora.exe').Path
function Show-Threads($p) {
    $p.Refresh()
    foreach ($t in $p.Threads) { Write-Host "  thread $($t.Id) state=$($t.ThreadState) wait=$($t.WaitReason) cpu=$($t.TotalProcessorTime)" }
}
Write-Host '=== 1. kora service allow, run directly, stdin closed, 40 s limit ==='
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$a = Start-Process $kora -ArgumentList "service allow $sid `"S-1-5-32-545 S-1-5-32-544`"" -PassThru -NoNewWindow -RedirectStandardOutput C:\probe1.out -RedirectStandardError C:\probe1.err -RedirectStandardInput NUL
if (-not $a.WaitForExit(40000)) { Write-Host 'HUNG:'; Show-Threads $a; Stop-Process -Id $a.Id -Force } else { Write-Host "exit $($a.ExitCode)" }
Get-Content C:\probe1.out, C:\probe1.err -ErrorAction SilentlyContinue | Write-Host
$f = "$env:ProgramFiles\Kora\data\service.json"; if (Test-Path $f) { Get-Content $f | Write-Host }
Remove-Item "$env:ProgramFiles\Kora" -Recurse -Force -ErrorAction SilentlyContinue

Write-Host '=== 2. the same as SYSTEM (scheduled task), 40 s limit ==='
$cmd = "& '$kora' service allow $sid 'S-1-5-32-545 S-1-5-32-544' > C:\probe2.out 2>&1; 'exit ' + `$LASTEXITCODE >> C:\probe2.out"
$act = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -Command `"$cmd`""
Register-ScheduledTask -TaskName KoraProbe -Action $act -User 'SYSTEM' -RunLevel Highest -Force | Out-Null
Start-ScheduledTask -TaskName KoraProbe
Start-Sleep 25
Get-Content C:\probe2.out -ErrorAction SilentlyContinue | Write-Host
Get-Process kora -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "STILL RUNNING kora pid $($_.Id) session $($_.SessionId)"; Show-Threads $_ ; Stop-Process -Id $_.Id -Force }
Unregister-ScheduledTask -TaskName KoraProbe -Confirm:$false
Remove-Item "$env:ProgramFiles\Kora" -Recurse -Force -ErrorAction SilentlyContinue

Write-Host '=== 3. the MSI with ALLOWUSERS, 120 s limit ==='
$m = Start-Process msiexec.exe -ArgumentList "/i `"$((Resolve-Path $Msi).Path)`" /qn /l*v C:\probe-msi.log ALLOWUSERS=`"S-1-5-32-545 S-1-5-32-544`"" -PassThru
if (-not $m.WaitForExit(120000)) {
    Write-Host 'MSI HUNG'
    Get-Process kora, msiexec -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "$($_.ProcessName) pid $($_.Id) session $($_.SessionId)"; Show-Threads $_ }
    Get-CimInstance Win32_Process -Filter "Name='kora.exe'" | ForEach-Object { "$($_.ProcessId) <- $($_.ParentProcessId): $($_.CommandLine)" } | Write-Host
    Get-Content C:\probe-msi.log -Tail 30 | Write-Host
    exit 1
}
Write-Host "msiexec exit $($m.ExitCode)"
sc.exe query KoraService | Write-Host
exit 0
