<#
  A tiny client for Kora's administrator service (smoke tests only; the panel has its own): one request over
  the named pipe, each progress line printed, then a final line.
    svc-client.ps1 -Verb admin-step -Arguments run,--step,system.firewall,--dir,C:\x
  Exit code: 0 = ran and succeeded, 3 = refused, 4 = ran and failed, 5 = no answer.
  The pipe is opened with exactly the rights the service grants (read data, write data, read attributes,
  read permissions, synchronize): GENERIC_WRITE would include "create pipe instance", which is never granted.
#>
param([Parameter(Mandatory)][string]$Verb, [string[]]$Arguments = @())
$ErrorActionPreference = 'Stop'
$rights = [System.IO.Pipes.PipeAccessRights]'ReadData, WriteData, ReadAttributes, ReadPermissions, Synchronize'
$c = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'kora-service', $rights, [System.IO.Pipes.PipeOptions]::None, [System.Security.Principal.TokenImpersonationLevel]::Identification, [System.IO.HandleInheritability]::None)
try { $c.Connect(5000) } catch { Write-Output "NO-ANSWER $($_.Exception.Message)"; exit 5 }
$reqEnv = @{}
if ($env:CFX_LICENSE_KEY) { $reqEnv.CFX_LICENSE_KEY = $env:CFX_LICENSE_KEY }
$req = @{ verb = $Verb; args = @($Arguments); env = $reqEnv } | ConvertTo-Json -Compress -Depth 4
$w = New-Object System.IO.StreamWriter($c, (New-Object System.Text.UTF8Encoding($false)))
$w.WriteLine($req); $w.Flush()
$r = New-Object System.IO.StreamReader($c, [System.Text.Encoding]::UTF8)
while ($null -ne ($line = $r.ReadLine())) {
    $m = $line | ConvertFrom-Json
    switch ($m.type) {
        'progress' { Write-Output $m.line }
        'result' { Write-Output ("RESULT ok=$("$($m.ok)".ToLower()) code=$($m.code)" + $(if ($m.message) { " message='$($m.message)'" } else { '' })); exit $(if ($m.ok) { 0 } else { 4 }) }
        'error' { Write-Output "REFUSED $($m.message)"; exit 3 }
    }
}
Write-Output 'NO-ANSWER connection closed'; exit 5
