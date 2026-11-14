. "$PSScriptRoot/common.ps1"
$kora = Get-KoraExe
$env:KORA_CATALOG = (Resolve-Path "$PWD/tests/fixtures/smoke-catalog.json").Path
$env:CFX_LICENSE_KEY = 'cfxk_FAKEFAKEFAKEFAKEFAKE_smoke'
$pd = Join-Path $env:ProgramData 'Kora\mariadb'

function Invoke-Sql([string]$Client, [int]$Port, [string]$User, [string]$Password, [string]$Sql, [string]$Database = '') {
    $env:MYSQL_PWD = $Password
    try {
        $a = @('--batch', '--skip-column-names', '--protocol=tcp', '-h', '127.0.0.1', '-P', "$Port", '-u', $User)
        if ($Database) { $a += $Database }
        $o = $Sql | & $Client @a 2>&1 | Out-String
        [pscustomobject]@{ Code = $LASTEXITCODE; Output = $o.Trim() }
    } finally { Remove-Item Env:MYSQL_PWD -ErrorAction SilentlyContinue }
}
function Read-DbName([string]$Dir) { (Get-Content "$Dir\database.json" -Raw | ConvertFrom-Json) }

# ============ A. an existing database server =================================================
# windows-latest no longer ships MySQL, so the "machine already has a server" case is built by hand: the
# official MariaDB zip, registered as the stock-looking service 'MariaDB' on 3306 with root password "root".
$stockRoot = 'C:\stock-mariadb'
$mysqlSvc = Get-Service -Name 'MySQL*' -ErrorAction SilentlyContinue | Select-Object -First 1
$stockPort = 3306
if (-not $mysqlSvc) {
    $ver = '11.4.14'
    $zipUrl = "https://archive.mariadb.org/mariadb-$ver/winx64-packages/mariadb-$ver-winx64.zip"
    $zip = Join-Path $env:RUNNER_TEMP 'stock-mariadb.zip'
    Invoke-WebRequest $zipUrl -OutFile $zip
    Expand-Archive $zip -DestinationPath 'C:\stock-extract' -Force
    Move-Item (Get-ChildItem 'C:\stock-extract' -Directory | Select-Object -First 1).FullName $stockRoot
    $r = Invoke-Native "$stockRoot\bin\mariadb-install-db.exe" @("--datadir=$stockRoot\data", '--service=MariaDB', '--password=root', '--port=3306')
    Show 'stock mariadb-install-db' $r
    Start-Service 'MariaDB'
    $mysqlSvc = Get-Service -Name 'MariaDB'
}
Write-Host "Existing server on this machine: '$($mysqlSvc.Name)' ($($mysqlSvc.Status))."
$client = (Get-ChildItem 'C:\Program Files\MySQL','C:\stock-mariadb\bin' -Recurse -Include mysql.exe,mariadb.exe -ErrorAction SilentlyContinue | Select-Object -First 1)
Assert-That ($null -ne $client) 'found a mysql/mariadb client'
if ($mysqlSvc.Status -ne 'Running') { Start-Service $mysqlSvc.Name }
$pw = $null
for ($i = 0; $i -lt 20 -and $null -eq $pw; $i++) {
    foreach ($try in @('root', '')) { if ((Invoke-Sql $client.FullName $stockPort 'root' $try 'SELECT 1').Code -eq 0) { $pw = $try; break } }
    if ($null -eq $pw) { Start-Sleep -Seconds 2 }
}
Assert-That ($null -ne $pw) 'the existing server accepts root (empty password or "root")'
$env:KORA_DB_ADMIN_PASSWORD = $pw
$d = 'C:\kora-smoke\dbx'
$r = Invoke-Native $kora @('install', '--name', 'dbx', '--dir', $d, '--game-port', '32120', '--txadmin-port', '42120', '--db', 'install', '--db-admin-user', 'root', '-y')
Show 'kora install --db install (existing server present)' $r
Assert-That ($r.Code -eq 0) 'install uses the existing server and exits 0'
Assert-That (-not (Test-Path $pd)) 'Kora did NOT install a second MariaDB next to the existing one'
$c = Read-DbName $d
Assert-That ($c.database -like 'kora_*' -and $c.user -like 'kora_*') "a NEW kora_ database + user were created ($($c.database))"
$x = Invoke-Sql $client.FullName $stockPort 'root' $pw "SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name='$($c.database)'"
Assert-That ($x.Output -eq '1') "the database exists on the server (got '$($x.Output)')"
$x = Invoke-Sql $client.FullName $stockPort $c.user $c.password 'SELECT 1' $c.database
Assert-That ($x.Code -eq 0) 'the generated login works'
$acl = Get-Acl-Text "$d\database.json"
Assert-That ($acl -notmatch '(?i)BUILTIN\\Users|Everyone|Authenticated Users') 'database.json ACL is restricted'
Assert-That ((Get-Content "$d\secrets.cfg" -Raw) -match 'mysql_connection_string') 'secrets.cfg has the connection string'
$r = Invoke-Native $kora @('delete', 'dbx', '-y'); Show 'kora delete dbx' $r
Assert-That ($r.Code -eq 0) 'kora delete exits 0'
$x = Invoke-Sql $client.FullName $stockPort 'root' $pw "SELECT (SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name='$($c.database)') + (SELECT COUNT(*) FROM mysql.user WHERE user='$($c.user)')"
Assert-That ($x.Output -eq '0') "the database AND its user are gone after kora delete (got '$($x.Output)')"
Assert-That (-not (Test-Path $d)) 'server folder gone'
Assert-That ((Get-Service $mysqlSvc.Name).Status -eq 'Running') 'the existing server itself was left alone'
# Remove the existing service so the next part is a machine with no database server at all.
Stop-Service $mysqlSvc.Name -Force -ErrorAction SilentlyContinue
& sc.exe delete $mysqlSvc.Name | Out-Null
Start-Sleep -Seconds 3
Remove-Item Env:KORA_DB_ADMIN_PASSWORD -ErrorAction SilentlyContinue

# ============ B. Kora installs its own MariaDB (fresh machine, --db install) ===========================
Assert-That (-not (Test-Path $pd)) 'no Kora MariaDB folder yet'
# Port 3306 is taken by something else: Kora's own MariaDB must pick the next free port.
$hold = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Any, 3306)
$hold.Start()
$d = 'C:\kora smoke ünï\db one'
$r = Invoke-Native $kora @('install', '--name', 'dbsmoke', '--dir', $d, '--game-port', '33120', '--txadmin-port', '43120', '--db', 'install', '-y')
Show 'kora install --db install' $r
Assert-That ($r.Code -eq 0) 'kora install --db install exits 0'
Assert-That ($r.Output -match 'MariaDB') 'the output mentions the MariaDB it installed'
$hold.Stop()
$rec = Get-Content "$pd\mariadb.json" -Raw | ConvertFrom-Json
Assert-That ($rec.port -ne 3306 -and $rec.port -ge 3307) "3306 was busy, so Kora's MariaDB uses port $($rec.port)"
$adm = Get-Content "$pd\database-admin.json" -Raw | ConvertFrom-Json
Assert-That ($rec.service -like 'KoraMariaDB_*') "unique service name ($($rec.service))"
$q = (& sc.exe query $rec.service | Out-String)
Assert-That ($q -match 'RUNNING') 'the MariaDB service exists and is RUNNING'
Assert-That ((& sc.exe qc $rec.service | Out-String) -match 'AUTO_START') 'it starts with Windows'
Assert-That (Test-Path $rec.client) 'mariadb.exe client is in the Kora folder'
Assert-That ($rec.root -like "$pd\*") 'installed under %ProgramData%\Kora\mariadb\<version>'
$listen = (& netstat -ano -p tcp | Select-String ":$($rec.port)\s" | Out-String)
Assert-That ($listen -match "127\.0\.0\.1:$($rec.port)" -and $listen -notmatch "0\.0\.0\.0:$($rec.port)") "MariaDB listens on 127.0.0.1:$($rec.port) only"
$acl = Get-Acl-Text "$pd\database-admin.json"; Write-Host $acl
Assert-That ($acl -notmatch '(?i)BUILTIN\\Users|Everyone|Authenticated Users') 'database-admin.json is not readable by Users'
$acl = Get-Acl-Text $pd
Assert-That ($acl -notmatch '(?i)BUILTIN\\Users|Everyone|Authenticated Users') 'the Kora MariaDB folder is not readable by Users'
$x = Invoke-Sql $rec.client $rec.port 'root' $adm.password.ToString() 'SELECT 1'
if ($x.Code -ne 0) { $x = Invoke-Sql $rec.client $rec.port 'root' $adm.password 'SELECT 1' }
Assert-That ($x.Code -eq 0) 'the saved root login works'
$c = Read-DbName $d
Assert-That ($c.database -like 'kora_*' -and $c.user -like 'kora_*' -and $c.port -eq $rec.port) "a kora_ database + user were created on the Kora MariaDB ($($c.database))"
$x = Invoke-Sql $rec.client $rec.port $c.user $c.password 'SELECT 1' $c.database
Assert-That ($x.Code -eq 0) 'the generated login works'
$x = Invoke-Sql $rec.client $rec.port 'root' $adm.password "SELECT COUNT(*) FROM mysql.user WHERE user='$($c.user)'"
Assert-That ($x.Output -eq '1') "the user exists server-side (got '$($x.Output)')"

# Running the install again reuses the same MariaDB (no second service).
$d2 = 'C:\kora smoke ünï\db two'
$r = Invoke-Native $kora @('install', '--name', 'dbsmoke2', '--dir', $d2, '--game-port', '34120', '--txadmin-port', '44120', '--db', 'install', '-y')
Assert-That ($r.Code -eq 0) 'a second server with --db install reuses the Kora MariaDB'
Assert-That (@(Get-Service 'KoraMariaDB_*').Count -eq 1) 'still exactly one Kora MariaDB service'
$c2 = Read-DbName $d2
Assert-That ($c2.database -ne $c.database) 'the second server got its own database'
$r = Invoke-Native $kora @('delete', 'dbsmoke2', '-y'); Show 'kora delete dbsmoke2' $r
Assert-That ($r.Code -eq 0) 'delete of the second server exits 0'

# kora delete drops the database and user but leaves the shared server alone.
$r = Invoke-Native $kora @('delete', 'dbsmoke', '-y'); Show 'kora delete dbsmoke' $r
Assert-That ($r.Code -eq 0) 'kora delete exits 0'
$x = Invoke-Sql $rec.client $rec.port 'root' $adm.password "SELECT (SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name IN ('$($c.database)','$($c2.database)')) + (SELECT COUNT(*) FROM mysql.user WHERE user IN ('$($c.user)','$($c2.user)'))"
Assert-That ($x.Output -eq '0') 'both databases AND their users are gone after kora delete'
Assert-That (-not (Test-Path $d)) 'server folder gone'
Assert-That ((& sc.exe query $rec.service | Out-String) -match 'RUNNING') 'the Kora MariaDB service itself is left running by kora delete'

# kora uninstall without --delete-databases keeps it; with it, removes service + folder.
$r = Invoke-Native $kora @('uninstall', '--yes'); Show 'kora uninstall --yes' $r
Assert-That ($r.Code -eq 0 -and (Test-Path $rec.root)) 'uninstall without --delete-databases keeps the MariaDB folder'
$r = Invoke-Native $kora @('uninstall', '--yes', '--delete-databases'); Show 'kora uninstall --yes --delete-databases' $r
Assert-That ($r.Code -eq 0) 'kora uninstall --delete-databases exits 0'
Assert-That ((& sc.exe query $rec.service | Out-String) -match '1060') 'the MariaDB service is gone'
Assert-That (-not (Test-Path $rec.root)) 'the MariaDB folder (program + data) is gone'
Assert-That (-not (Test-Path "$pd\database-admin.json")) 'the saved admin login is gone'

exit 0
