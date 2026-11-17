# Helpers for tests that need more than the built-in elevated CI account: local test accounts, the UAC policy,
# token diagnostics, a process under a FILTERED (unelevated administrator) token, a process as another account,
# and a panel started that way and driven through its HTTP API.
#
# Dot-source: . scripts/smoke/lib-accounts.ps1      (needs an elevated PowerShell; it loads common.ps1)
#
# Written to be reused for the installed privileged service later: run the service's client (or the panel) as
#   Kind 'Filtered' : the same administrator, but unelevated (deny-only Administrators group, medium integrity)
#   Kind 'Account'  : another local account (standard, or administrator), through the secondary logon service
#   Kind 'Self'     : the current process token (use this on a runner that is itself an unelevated administrator)
# and assert what must be refused / allowed.
#
# Nothing here has been run on Windows by its author; every assumption about Windows is listed in
# docs/windows-uac-ci.md ("Assumptions to check on the first runs"). Functions avoid the verbs New/Set/Remove/Start/
# Stop on purpose so PSScriptAnalyzer does not ask for ShouldProcess support.
. "$PSScriptRoot/common.ps1"

if ($PSVersionTable.PSVersion.Major -lt 5) { throw 'PowerShell 5.1 or newer is needed' }

# ---------------------------------------------------------------------------------------------------------------
# Token access (P/Invoke). C# 5 syntax only: Windows PowerShell 5.1 compiles this with the old compiler.
# ---------------------------------------------------------------------------------------------------------------
if (-not ('KoraTok' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

public static class KoraTok
{
    [StructLayout(LayoutKind.Sequential)]
    struct LUID { public uint Low; public int High; }

    [StructLayout(LayoutKind.Sequential)]
    struct SID_AND_ATTRIBUTES { public IntPtr Sid; public uint Attributes; }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct STARTUPINFO
    {
        public int cb;
        public string lpReserved;
        public string lpDesktop;
        public string lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow, cbReserved2;
        public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct PROCESS_INFORMATION { public IntPtr hProcess, hThread; public int dwProcessId, dwThreadId; }

    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr p);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool GetTokenInformation(IntPtr token, int cls, IntPtr buf, int len, out int ret);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool SetTokenInformation(IntPtr token, int cls, IntPtr buf, int len);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool ConvertSidToStringSid(IntPtr sid, out IntPtr str);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool ConvertStringSidToSid(string s, out IntPtr sid);
    [DllImport("advapi32.dll")] static extern int GetLengthSid(IntPtr sid);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool LookupAccountSid(string system, IntPtr sid, StringBuilder name, ref int cchName, StringBuilder domain, ref int cchDomain, out int use);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool LookupPrivilegeName(string system, ref LUID luid, StringBuilder name, ref int cch);
    [DllImport("advapi32.dll", SetLastError = true)]
    static extern bool CreateRestrictedToken(IntPtr existing, uint flags, uint disableCount, IntPtr sidsToDisable, uint deleteCount, IntPtr privsToDelete, uint restrictCount, IntPtr sidsToRestrict, out IntPtr newToken);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool CreateProcessAsUser(IntPtr token, string app, StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string cwd, ref STARTUPINFO si, out PROCESS_INFORMATION pi);

    const uint TOKEN_ASSIGN_PRIMARY = 0x1, TOKEN_DUPLICATE = 0x2, TOKEN_QUERY = 0x8, TOKEN_ADJUST_DEFAULT = 0x80;
    const uint PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;
    const int TokenUser = 1, TokenGroups = 2, TokenPrivileges = 3, TokenOwner = 4, TokenSessionId = 12, TokenElevationTypeClass = 18, TokenElevationClass = 20, TokenIntegrityLevel = 25;

    static IntPtr OpenToken(int pid, uint access)
    {
        IntPtr proc;
        if (pid <= 0) { proc = GetCurrentProcess(); }
        else
        {
            proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
            if (proc == IntPtr.Zero) { throw new Win32Exception(Marshal.GetLastWin32Error(), "OpenProcess " + pid); }
        }
        IntPtr tok;
        bool ok = OpenProcessToken(proc, access, out tok);
        int err = Marshal.GetLastWin32Error();
        if (pid > 0) { CloseHandle(proc); }
        if (!ok) { throw new Win32Exception(err, "OpenProcessToken " + pid); }
        return tok;
    }

    // Caller frees with Marshal.FreeHGlobal.
    static IntPtr Info(IntPtr tok, int cls)
    {
        int need;
        GetTokenInformation(tok, cls, IntPtr.Zero, 0, out need);
        if (need <= 0) { throw new Win32Exception(Marshal.GetLastWin32Error(), "GetTokenInformation " + cls); }
        IntPtr buf = Marshal.AllocHGlobal(need);
        if (!GetTokenInformation(tok, cls, buf, need, out need))
        {
            int err = Marshal.GetLastWin32Error();
            Marshal.FreeHGlobal(buf);
            throw new Win32Exception(err, "GetTokenInformation " + cls);
        }
        return buf;
    }

    static string SidText(IntPtr sid)
    {
        IntPtr s;
        if (!ConvertSidToStringSid(sid, out s)) { return "?"; }
        string r = Marshal.PtrToStringUni(s);
        LocalFree(s);
        return r;
    }

    static string SidName(IntPtr sid)
    {
        StringBuilder n = new StringBuilder(256), d = new StringBuilder(256);
        int cn = 256, cd = 256, use;
        if (!LookupAccountSid(null, sid, n, ref cn, d, ref cd, out use)) { return ""; }
        return d.ToString() + "\\" + n.ToString();
    }

    static int ReadInt(IntPtr tok, int cls)
    {
        IntPtr b = Info(tok, cls);
        try { return Marshal.ReadInt32(b); } finally { Marshal.FreeHGlobal(b); }
    }

    // 1 = default (no split token), 2 = full (elevated), 3 = limited (the filtered half of a split token).
    public static int ElevationType(int pid)
    {
        IntPtr t = OpenToken(pid, TOKEN_QUERY);
        try { return ReadInt(t, TokenElevationTypeClass); } finally { CloseHandle(t); }
    }

    public static bool IsElevated(int pid)
    {
        IntPtr t = OpenToken(pid, TOKEN_QUERY);
        try { return ReadInt(t, TokenElevationClass) != 0; } finally { CloseHandle(t); }
    }

    // The integrity RID: 4096 low, 8192 medium, 12288 high, 16384 system.
    public static int Integrity(int pid)
    {
        IntPtr t = OpenToken(pid, TOKEN_QUERY);
        try
        {
            IntPtr b = Info(t, TokenIntegrityLevel);
            try
            {
                SID_AND_ATTRIBUTES sa = (SID_AND_ATTRIBUTES)Marshal.PtrToStructure(b, typeof(SID_AND_ATTRIBUTES));
                string[] parts = SidText(sa.Sid).Split('-');
                return int.Parse(parts[parts.Length - 1]);
            }
            finally { Marshal.FreeHGlobal(b); }
        }
        finally { CloseHandle(t); }
    }

    // A readable dump of a process token (pid 0 = this process). Never throws: failures are part of the text.
    public static string Describe(int pid)
    {
        StringBuilder sb = new StringBuilder();
        IntPtr t = IntPtr.Zero;
        try
        {
            t = OpenToken(pid, TOKEN_QUERY);
            IntPtr b = Info(t, TokenUser);
            try { SID_AND_ATTRIBUTES u = (SID_AND_ATTRIBUTES)Marshal.PtrToStructure(b, typeof(SID_AND_ATTRIBUTES)); sb.AppendLine("user: " + SidName(u.Sid) + " " + SidText(u.Sid)); }
            finally { Marshal.FreeHGlobal(b); }
            int et = ReadInt(t, TokenElevationTypeClass);
            sb.AppendLine("elevation type: " + et + (et == 1 ? " (default: no split token)" : et == 2 ? " (full: elevated)" : et == 3 ? " (limited: filtered half of a split token)" : ""));
            sb.AppendLine("elevated: " + (ReadInt(t, TokenElevationClass) != 0));
            sb.AppendLine("session: " + ReadInt(t, TokenSessionId));
            sb.AppendLine("integrity: " + Integrity(pid));
            IntPtr g = Info(t, TokenGroups);
            try
            {
                int n = Marshal.ReadInt32(g);
                int step = 2 * IntPtr.Size;
                for (int i = 0; i < n; i++)
                {
                    IntPtr e = new IntPtr(g.ToInt64() + IntPtr.Size + i * step);
                    SID_AND_ATTRIBUTES a = (SID_AND_ATTRIBUTES)Marshal.PtrToStructure(e, typeof(SID_AND_ATTRIBUTES));
                    string flags = ((a.Attributes & 0x4) != 0 ? "enabled " : "") + ((a.Attributes & 0x10) != 0 ? "DENY-ONLY " : "") + ((a.Attributes & 0x20) != 0 ? "integrity " : "") + ((a.Attributes & 0x8) != 0 ? "owner " : "");
                    sb.AppendLine("  group " + SidText(a.Sid) + " " + SidName(a.Sid) + " [" + flags.Trim() + "]");
                }
            }
            finally { Marshal.FreeHGlobal(g); }
            IntPtr p = Info(t, TokenPrivileges);
            try
            {
                int n = Marshal.ReadInt32(p);
                for (int i = 0; i < n; i++)
                {
                    IntPtr e = new IntPtr(p.ToInt64() + 4 + i * 12);
                    LUID l = (LUID)Marshal.PtrToStructure(e, typeof(LUID));
                    uint attr = (uint)Marshal.ReadInt32(e, 8);
                    StringBuilder nm = new StringBuilder(128);
                    int cch = 128;
                    LookupPrivilegeName(null, ref l, nm, ref cch);
                    sb.AppendLine("  privilege " + nm + ((attr & 0x2) != 0 ? " [enabled]" : " [disabled]"));
                }
            }
            finally { Marshal.FreeHGlobal(p); }
        }
        catch (Exception ex) { sb.AppendLine("cannot read the token of pid " + pid + ": " + ex.Message); }
        finally { if (t != IntPtr.Zero) { CloseHandle(t); } }
        return sb.ToString();
    }

    // What UAC gives an administrator who is not elevated, made by hand: the Administrators group becomes
    // deny-only, every privilege except SeChangeNotify is dropped, integrity is medium, the owner is the user.
    // Starts `commandLine` with it and returns the new process id. Needs no special privilege: it is a restricted
    // copy of the caller's own primary token.
    public static int LaunchFiltered(string exe, string commandLine, string cwd)
    {
        IntPtr src = OpenToken(0, TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY);
        IntPtr admins = IntPtr.Zero, medium = IntPtr.Zero, disable = IntPtr.Zero, labelBuf = IntPtr.Zero, ownerBuf = IntPtr.Zero, userInfo = IntPtr.Zero, tok = IntPtr.Zero;
        try
        {
            if (!ConvertStringSidToSid("S-1-5-32-544", out admins)) { throw new Win32Exception(Marshal.GetLastWin32Error(), "ConvertStringSidToSid admins"); }
            SID_AND_ATTRIBUTES dis = new SID_AND_ATTRIBUTES();
            dis.Sid = admins;
            disable = Marshal.AllocHGlobal(Marshal.SizeOf(typeof(SID_AND_ATTRIBUTES)));
            Marshal.StructureToPtr(dis, disable, false);
            // DISABLE_MAX_PRIVILEGE (0x1) | LUA_TOKEN (0x4)
            if (!CreateRestrictedToken(src, 0x1 | 0x4, 1, disable, 0, IntPtr.Zero, 0, IntPtr.Zero, out tok)) { throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateRestrictedToken"); }

            if (!ConvertStringSidToSid("S-1-16-8192", out medium)) { throw new Win32Exception(Marshal.GetLastWin32Error(), "ConvertStringSidToSid medium"); }
            SID_AND_ATTRIBUTES lab = new SID_AND_ATTRIBUTES();
            lab.Sid = medium;
            lab.Attributes = 0x20; // SE_GROUP_INTEGRITY
            labelBuf = Marshal.AllocHGlobal(Marshal.SizeOf(typeof(SID_AND_ATTRIBUTES)));
            Marshal.StructureToPtr(lab, labelBuf, false);
            if (!SetTokenInformation(tok, TokenIntegrityLevel, labelBuf, Marshal.SizeOf(typeof(SID_AND_ATTRIBUTES)) + GetLengthSid(medium))) { throw new Win32Exception(Marshal.GetLastWin32Error(), "SetTokenInformation integrity"); }

            userInfo = Info(src, TokenUser);
            SID_AND_ATTRIBUTES me = (SID_AND_ATTRIBUTES)Marshal.PtrToStructure(userInfo, typeof(SID_AND_ATTRIBUTES));
            ownerBuf = Marshal.AllocHGlobal(IntPtr.Size);
            Marshal.WriteIntPtr(ownerBuf, me.Sid);
            if (!SetTokenInformation(tok, TokenOwner, ownerBuf, IntPtr.Size)) { throw new Win32Exception(Marshal.GetLastWin32Error(), "SetTokenInformation owner"); }

            STARTUPINFO si = new STARTUPINFO();
            si.cb = Marshal.SizeOf(typeof(STARTUPINFO));
            PROCESS_INFORMATION pi;
            StringBuilder cmd = new StringBuilder(commandLine);
            // CREATE_NO_WINDOW
            if (!CreateProcessAsUser(tok, exe, cmd, IntPtr.Zero, IntPtr.Zero, false, 0x08000000, IntPtr.Zero, cwd, ref si, out pi)) { throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateProcessAsUser"); }
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
            return pi.dwProcessId;
        }
        finally
        {
            if (admins != IntPtr.Zero) { LocalFree(admins); }
            if (medium != IntPtr.Zero) { LocalFree(medium); }
            if (disable != IntPtr.Zero) { Marshal.FreeHGlobal(disable); }
            if (labelBuf != IntPtr.Zero) { Marshal.FreeHGlobal(labelBuf); }
            if (ownerBuf != IntPtr.Zero) { Marshal.FreeHGlobal(ownerBuf); }
            if (userInfo != IntPtr.Zero) { Marshal.FreeHGlobal(userInfo); }
            if (tok != IntPtr.Zero) { CloseHandle(tok); }
            CloseHandle(src);
        }
    }
}
'@
}

# ---------------------------------------------------------------------------------------------------------------
# Diagnostics
# ---------------------------------------------------------------------------------------------------------------

# The elevation facts of a process as one object (for assertions) ...
function Get-TokenFact([int]$ProcessId = 0) {
    [pscustomobject]@{
        ProcessId     = $ProcessId
        ElevationType = [KoraTok]::ElevationType($ProcessId)
        Elevated      = [KoraTok]::IsElevated($ProcessId)
        Integrity     = [KoraTok]::Integrity($ProcessId)
    }
}

# ... and as a dump for the log.
function Write-TokenDiagnostic([string]$Title, [int]$ProcessId = 0) {
    Write-Host "::group::token: $Title (pid $ProcessId, 0 = this process)"
    Write-Host ([KoraTok]::Describe($ProcessId))
    Write-Host '::endgroup::'
}

function Get-UacPolicy {
    $k = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System'
    $o = [ordered]@{}
    foreach ($n in 'EnableLUA', 'ConsentPromptBehaviorAdmin', 'ConsentPromptBehaviorUser', 'PromptOnSecureDesktop', 'FilterAdministratorToken', 'LocalAccountTokenFilterPolicy', 'EnableInstallerDetection') {
        $v = Get-ItemProperty -Path $k -Name $n -ErrorAction SilentlyContinue
        $o[$n] = if ($null -ne $v) { $v.$n } else { $null }
    }
    [pscustomobject]$o
}

# Change the policy values that take effect WITHOUT a restart (everything but EnableLUA, which needs one).
# Returns what was there before, for Restore-UacPolicy. $null = leave as it is.
#   ConsentPromptBehaviorAdmin: 0 elevate without prompting, 1 credentials on secure desktop, 2 consent on secure
#                               desktop, 3 credentials, 4 consent, 5 consent for non-Windows binaries (default)
#   ConsentPromptBehaviorUser : 0 automatically deny, 1 credentials on secure desktop, 3 credentials (default)
#   PromptOnSecureDesktop     : 1 (default) the prompt is on the secure desktop; 0 on the normal desktop, where UI
#                               automation can reach it
function Initialize-UacPolicy($ConsentPromptBehaviorAdmin = $null, $ConsentPromptBehaviorUser = $null, $PromptOnSecureDesktop = $null) {
    $k = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System'
    $before = Get-UacPolicy
    $want = @{ ConsentPromptBehaviorAdmin = $ConsentPromptBehaviorAdmin; ConsentPromptBehaviorUser = $ConsentPromptBehaviorUser; PromptOnSecureDesktop = $PromptOnSecureDesktop }
    foreach ($n in $want.Keys) {
        if ($null -ne $want[$n]) { Set-ItemProperty -Path $k -Name $n -Value ([int]$want[$n]) -Type DWord }
    }
    Write-Host ('UAC policy now: ' + ((Get-UacPolicy | Out-String).Trim() -replace '\s*\r?\n\s*', '; '))
    $before
}

function Restore-UacPolicy($Before) {
    if ($null -eq $Before) { return }
    $k = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System'
    foreach ($n in 'ConsentPromptBehaviorAdmin', 'ConsentPromptBehaviorUser', 'PromptOnSecureDesktop') {
        $v = $Before.$n
        if ($null -eq $v) { Remove-ItemProperty -Path $k -Name $n -ErrorAction SilentlyContinue } else { Set-ItemProperty -Path $k -Name $n -Value ([int]$v) -Type DWord }
    }
}

# Kora processes with their command lines and owners (the process tree is what matters when a worker is left).
function Get-KoraProcess {
    $all = @(Get-CimInstance Win32_Process)
    $byPid = @{}
    foreach ($p in $all) { $byPid[[int]$p.ProcessId] = $p }
    foreach ($p in ($all | Where-Object { $_.Name -in 'kora.exe', 'consent.exe', 'cmd.exe' -or $_.CommandLine -match 'kora' })) {
        if ($p.Name -eq 'cmd.exe' -and $p.CommandLine -notmatch 'kora') { continue }
        $parent = $byPid[[int]$p.ParentProcessId]
        [pscustomobject]@{
            ProcessId  = [int]$p.ProcessId
            Parent     = [int]$p.ParentProcessId
            ParentName = if ($parent) { $parent.Name } else { '(gone)' }
            Name       = $p.Name
            Session    = [int]$p.SessionId
            CommandLine = $p.CommandLine
        }
    }
}

function Write-KoraProcessDiagnostic {
    Write-Host '::group::kora processes (and consent.exe)'
    $l = @(Get-KoraProcess)
    if ($l.Count -eq 0) { Write-Host '(none)' } else { Write-Host ($l | Format-Table -AutoSize -Wrap | Out-String -Width 300) }
    Write-Host '::endgroup::'
}

# Every kora-admin pipe and, where Windows lets us read it, its security descriptor.
function Write-PipeDiagnostic {
    Write-Host '::group::named pipes kora-admin-*'
    $names = @([System.IO.Directory]::GetFiles('\\.\pipe\') | Where-Object { $_ -match 'kora-admin' })
    if ($names.Count -eq 0) { Write-Host '(none)' }
    foreach ($n in $names) {
        Write-Host $n
        try { Write-Host ((Get-Acl -LiteralPath $n).Sddl) } catch { Write-Host "  (cannot read its ACL: $($_.Exception.Message))" }
    }
    Write-Host '::endgroup::'
}

# Policy, accounts, tokens, processes, pipes and the event logs' tail: everything to read when a run fails.
function Write-SystemDiagnostic([string]$Title = 'diagnostics') {
    Write-Host "::group::$Title"
    try {
        Write-Host (Get-UacPolicy | Format-List | Out-String)
        Write-Host (& whoami.exe /all 2>&1 | Out-String)
        Write-Host ((& net.exe localgroup Administrators 2>&1) | Out-String)
        Write-Host ((Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber | Format-List | Out-String))
        Write-Host ('secondary logon service: ' + ((Get-Service seclogon -ErrorAction SilentlyContinue).Status))
        Write-Host ('AppInfo (UAC) service: ' + ((Get-Service Appinfo -ErrorAction SilentlyContinue).Status))
    }
    catch { Write-Host "(diagnostics partly failed: $($_.Exception.Message))" }
    Write-Host '::endgroup::'
    Write-TokenDiagnostic 'this process'
    Write-KoraProcessDiagnostic
    Write-PipeDiagnostic
    foreach ($log in 'System', 'Application') {
        Write-Host "::group::event log $log (last 25, warnings and errors, plus anything from UAC/AppInfo/seclogon)"
        try {
            $ev = Get-WinEvent -LogName $log -MaxEvents 200 -ErrorAction Stop | Where-Object { $_.Level -le 3 -or $_.ProviderName -match 'Appinfo|User Account Control|seclogon|Service Control' } | Select-Object -First 25
            foreach ($e in $ev) { Write-Host ('{0:s} {1} [{2}] {3}' -f $e.TimeCreated, $e.ProviderName, $e.Id, ($e.Message -replace '\s+', ' ')) }
        }
        catch { Write-Host "(cannot read: $($_.Exception.Message))" }
        Write-Host '::endgroup::'
    }
}

# ---------------------------------------------------------------------------------------------------------------
# Accounts
# ---------------------------------------------------------------------------------------------------------------

# A random password that satisfies the default complexity rules. Masked in the Actions log at once.
function Get-RandomSecret([int]$Length = 24) {
    $upper = 'ABCDEFGHJKLMNPQRSTUVWXYZ'; $lower = 'abcdefghijkmnopqrstuvwxyz'; $digit = '23456789'; $punct = '#%+=_'
    $all = $upper + $lower + $digit
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    $pick = {
        param([string]$set)
        $b = New-Object byte[] 4
        $rng.GetBytes($b)
        $set[[int]([BitConverter]::ToUInt32($b, 0) % $set.Length)]
    }
    $chars = New-Object System.Collections.Generic.List[char]
    foreach ($set in $upper, $lower, $digit, $punct) { $chars.Add((& $pick $set)) }
    while ($chars.Count -lt $Length) { $chars.Add((& $pick $all)) }
    $rng.Dispose()
    # shuffle (Fisher-Yates with the same generator type)
    $r2 = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    for ($i = $chars.Count - 1; $i -gt 0; $i--) {
        $b = New-Object byte[] 4
        $r2.GetBytes($b)
        $j = [int]([BitConverter]::ToUInt32($b, 0) % ($i + 1))
        $t = $chars[$i]; $chars[$i] = $chars[$j]; $chars[$j] = $t
    }
    $r2.Dispose()
    $s = -join $chars
    if ($env:GITHUB_ACTIONS -eq 'true') { Write-Host "::add-mask::$s" }
    $s
}

function ConvertTo-SecretString([string]$Text) {
    $s = New-Object System.Security.SecureString
    foreach ($c in $Text.ToCharArray()) { $s.AppendChar($c) }
    $s.MakeReadOnly()
    $s
}

function Get-SecretText([System.Security.SecureString]$Secret) {
    $p = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($Secret)
    try { [System.Runtime.InteropServices.Marshal]::PtrToStringBSTR($p) } finally { [System.Runtime.InteropServices.Marshal]::ZeroFreeBSTR($p) }
}

# A local account. -Administrator adds it to the Administrators group. Returns Name, Credential (PSCredential,
# for Start-Process -Credential), Sid. `net user` instead of the LocalAccounts module: that module does not load
# in PowerShell 7 without the compatibility layer.
function Initialize-TestAccount([Parameter(Mandatory)][string]$Name, [switch]$Administrator) {
    if ($Name.Length -gt 20 -or $Name -notmatch '^[A-Za-z][A-Za-z0-9_-]*$') { throw "bad test account name '$Name'" }
    $plain = Get-RandomSecret
    $null = Invoke-Native 'net.exe' @('user', $Name, '/delete')   # a leftover from an earlier run (an error here is fine)
    $r = Invoke-Native 'net.exe' @('user', $Name, $plain, '/add', '/expires:never', '/passwordchg:no', '/comment:Kora CI test account')
    if ($r.Code -ne 0) { throw "net user /add $Name failed: $($r.Output)" }
    if ($Administrator) {
        $r = Invoke-Native 'net.exe' @('localgroup', 'Administrators', $Name, '/add')
        if ($r.Code -ne 0) { throw "net localgroup Administrators $Name /add failed: $($r.Output)" }
    }
    $sid = (New-Object System.Security.Principal.NTAccount($env:COMPUTERNAME, $Name)).Translate([System.Security.Principal.SecurityIdentifier]).Value
    $cred = New-Object System.Management.Automation.PSCredential("$env:COMPUTERNAME\$Name", (ConvertTo-SecretString $plain))
    Write-Host ("test account {0} ({1}) created, administrator={2}" -f $Name, $sid, [bool]$Administrator)
    [pscustomobject]@{ Name = $Name; Sid = $sid; Credential = $cred; Administrator = [bool]$Administrator }
}

# Delete the account, its profile folder and anything of it still running.
function Clear-TestAccount([Parameter(Mandatory)][string]$Name) {
    Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | ForEach-Object {
        $o = Invoke-CimMethod -InputObject $_ -MethodName GetOwner -ErrorAction SilentlyContinue
        if ($o -and $o.User -eq $Name) { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    }
    $userProfile = Get-CimInstance Win32_UserProfile -ErrorAction SilentlyContinue | Where-Object { $_.LocalPath -like "*\$Name" }
    $null = Invoke-Native 'net.exe' @('user', $Name, '/delete')
    if ($userProfile) { $userProfile | Remove-CimInstance -ErrorAction SilentlyContinue }
}

# ---------------------------------------------------------------------------------------------------------------
# A working folder every test account can use (and only administrators and those accounts), and the Kora program
# copied to a folder ordinary users cannot write to: that is what makes the panel ask for administrator rights
# to update it (`self_update_needs_admin`).
# ---------------------------------------------------------------------------------------------------------------
function Initialize-UacWorkDir([string]$Path = 'C:\kora-uac') {
    if (Test-Path $Path) { Remove-Item $Path -Recurse -Force -ErrorAction SilentlyContinue }
    $null = New-Item -ItemType Directory -Force -Path $Path
    $null = Invoke-Native 'icacls.exe' @($Path, '/grant', '*S-1-5-32-545:(OI)(CI)M')   # BUILTIN\Users may add and change files here
    $Path
}

# Copy kora.exe to %ProgramFiles%\KoraUacTest\ (inherits: Users read and execute only).
function Initialize-ProtectedKora([string]$Source, [string]$Name = 'KoraUacTest') {
    $dir = Join-Path $env:ProgramFiles $Name
    if (Test-Path $dir) { Remove-Item $dir -Recurse -Force }
    $null = New-Item -ItemType Directory -Force -Path $dir
    Copy-Item $Source (Join-Path $dir 'kora.exe')
    $exe = Join-Path $dir 'kora.exe'
    Write-Host ((Invoke-Native 'icacls.exe' @($dir)).Output.Trim())
    $exe
}

# ---------------------------------------------------------------------------------------------------------------
# Starting the panel as something other than this (elevated) process, and talking to it.
# ---------------------------------------------------------------------------------------------------------------

function ConvertTo-PsLiteral([string]$Text) { "'" + ($Text -replace "'", "''") + "'" }

# The script run under the other token. It starts `kora panel` there, copies the panel's sign-in record (owner-only
# on disk, so this process could not read it) to a file the test can read, waits for a `stop` file and ends the panel.
function Initialize-PanelWrapper([string]$Dir, [string]$Tag, [string]$Exe, [int]$Port, [hashtable]$ExtraEnv = @{}) {
    $homeDir = Join-Path $Dir "home-$Tag"
    $null = New-Item -ItemType Directory -Force -Path $homeDir
    $envLines = ($ExtraEnv.Keys | ForEach-Object { "`$env:$_ = $(ConvertTo-PsLiteral $ExtraEnv[$_])" }) -join "`r`n"
    $script = @"
`$ErrorActionPreference = 'Continue'
`$env:KORA_HOME = $(ConvertTo-PsLiteral $homeDir)
$envLines
`$log = $(ConvertTo-PsLiteral (Join-Path $Dir "panel-$Tag.log"))
`$info = $(ConvertTo-PsLiteral (Join-Path $Dir "panel-$Tag.json"))
`$stop = $(ConvertTo-PsLiteral (Join-Path $Dir "panel-$Tag.stop"))
"whoami: " + [Environment]::UserName | Set-Content `$log
`$p = Start-Process -FilePath $(ConvertTo-PsLiteral $Exe) -ArgumentList @('panel', '--foreground', '--no-open', '--port', '$Port') -PassThru -WindowStyle Hidden -RedirectStandardOutput ($(ConvertTo-PsLiteral (Join-Path $Dir "panel-$Tag.out"))) -RedirectStandardError ($(ConvertTo-PsLiteral (Join-Path $Dir "panel-$Tag.err")))
`$rec = Join-Path $(ConvertTo-PsLiteral $homeDir) 'panel-running.json'
for (`$i = 0; `$i -lt 120 -and -not (Test-Path `$rec); `$i++) { Start-Sleep -Milliseconds 500 }
if (Test-Path `$rec) { Copy-Item `$rec `$info -Force } else { 'no panel-running.json' | Add-Content `$log }
while (-not (Test-Path `$stop) -and -not `$p.HasExited) { Start-Sleep -Milliseconds 300 }
if (-not `$p.HasExited) { Stop-Process -Id `$p.Id -Force }
"panel exit: " + `$p.ExitCode | Add-Content `$log
"@
    $path = Join-Path $Dir "panel-$Tag.ps1"
    Set-Content -Path $path -Value $script -Encoding ASCII
    [pscustomobject]@{ Script = $path; Home = $homeDir; Info = (Join-Path $Dir "panel-$Tag.json"); Stop = (Join-Path $Dir "panel-$Tag.stop"); Log = (Join-Path $Dir "panel-$Tag.log"); Out = (Join-Path $Dir "panel-$Tag.out"); Err = (Join-Path $Dir "panel-$Tag.err") }
}

# Start `kora panel` and wait until it is signed in-able. Kind:
#   Self     the current token
#   Filtered this administrator, filtered (see KoraTok.LaunchFiltered)
#   Account  the account in -Credential (secondary logon; a split token if it is an administrator and UAC is on)
# Returns an object for Invoke-PanelApi / Close-KoraPanel; WrapperProcessId is the process that runs the wrapper.
function Open-KoraPanel([ValidateSet('Self', 'Filtered', 'Account')][string]$Kind, [string]$Exe, [string]$Dir, [string]$Tag, [int]$Port, [pscredential]$Credential, [hashtable]$ExtraEnv = @{}, [int]$TimeoutSec = 90) {
    $w = Initialize-PanelWrapper -Dir $Dir -Tag $Tag -Exe $Exe -Port $Port -ExtraEnv $ExtraEnv
    $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $argText = "-NoProfile -ExecutionPolicy Bypass -File `"$($w.Script)`""
    switch ($Kind) {
        'Self' { $proc = Start-Process -FilePath $ps -ArgumentList $argText -PassThru -WindowStyle Hidden; $wrapperPid = $proc.Id }
        'Filtered' { $wrapperPid = [KoraTok]::LaunchFiltered($ps, "`"$ps`" $argText", $Dir) }
        'Account' {
            if (-not $Credential) { throw 'Kind Account needs -Credential' }
            $svc = Get-Service seclogon -ErrorAction SilentlyContinue
            if ($svc -and $svc.Status -ne 'Running') { Start-Service seclogon }
            $proc = Start-Process -FilePath $ps -ArgumentList $argText -Credential $Credential -LoadUserProfile -WorkingDirectory $Dir -PassThru -WindowStyle Hidden
            $wrapperPid = $proc.Id
        }
    }
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path $w.Info) -and $sw.Elapsed.TotalSeconds -lt $TimeoutSec) { Start-Sleep -Milliseconds 500 }
    if (-not (Test-Path $w.Info)) {
        foreach ($f in $w.Log, $w.Out, $w.Err) { if (Test-Path $f) { Write-Host "== $f"; Get-Content $f -Tail 40 } }
        throw "the panel ($Tag) did not start within $TimeoutSec s"
    }
    $rec = Get-Content $w.Info -Raw | ConvertFrom-Json
    $panelPid = [int]$rec.pid
    Write-TokenDiagnostic "panel '$Tag'" $panelPid
    [pscustomobject]@{ Tag = $Tag; Port = [int]$rec.port; Token = [string]$rec.token; PanelProcessId = $panelPid; WrapperProcessId = $wrapperPid; Files = $w; Kind = $Kind }
}

function Close-KoraPanel($Panel) {
    if ($null -eq $Panel) { return }
    Set-Content -Path $Panel.Files.Stop -Value 'stop'
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ((Get-Process -Id $Panel.PanelProcessId -ErrorAction SilentlyContinue) -and $sw.Elapsed.TotalSeconds -lt 15) { Start-Sleep -Milliseconds 300 }
    Stop-Process -Id $Panel.PanelProcessId -Force -ErrorAction SilentlyContinue
    Stop-Process -Id $Panel.WrapperProcessId -Force -ErrorAction SilentlyContinue
}

# One call to the panel's API with the launch token (a local caller may send it as a Bearer token). Returns
# Status, Body (parsed JSON or $null) and Raw; never throws on an HTTP error status.
function Invoke-PanelApi($Panel, [string]$Method, [string]$Path, $Body = $null) {
    $uri = "http://127.0.0.1:$($Panel.Port)/api/v1$Path"
    $req = @{ Uri = $uri; Method = $Method; Headers = @{ Authorization = "Bearer $($Panel.Token)" }; UseBasicParsing = $true; TimeoutSec = 30 }
    if ($null -ne $Body) { $req.Body = ($Body | ConvertTo-Json -Compress); $req.ContentType = 'application/json' }
    try {
        $r = Invoke-WebRequest @req
        $status = [int]$r.StatusCode; $raw = [string]$r.Content
    }
    catch {
        $resp = $_.Exception.Response
        if ($null -eq $resp) { return [pscustomobject]@{ Status = 0; Body = $null; Raw = $_.Exception.Message } }
        $status = [int]$resp.StatusCode
        $raw = ''
        try { $raw = (New-Object System.IO.StreamReader($resp.GetResponseStream())).ReadToEnd() } catch { $raw = $_.Exception.Message }
        if ($resp.PSObject.Properties.Name -contains 'Content' -and $resp.Content) { $raw = $resp.Content.ReadAsStringAsync().Result }
    }
    $parsed = $null
    try { $parsed = $raw | ConvertFrom-Json } catch { Write-Verbose "not JSON: $raw" }
    [pscustomobject]@{ Status = $status; Body = $parsed; Raw = $raw }
}

# Follow a job until it ends or `-TimeoutSec`; `-OnTick` runs about every 200 ms (to watch processes and pipes
# while the administrator step is going on). Returns the job object ({state, events, result, error}) or $null.
function Wait-PanelJob($Panel, [string]$JobId, [int]$TimeoutSec = 120, [scriptblock]$OnTick = $null) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $job = $null
    while ($sw.Elapsed.TotalSeconds -lt $TimeoutSec) {
        if ($OnTick) { & $OnTick }
        $r = Invoke-PanelApi -Panel $Panel -Method 'GET' -Path "/jobs/$JobId"
        if ($r.Status -eq 200 -and $r.Body) {
            $job = $r.Body
            if ($job.state -ne 'running') { return $job }
        }
        Start-Sleep -Milliseconds 200
    }
    $job
}

# ---------------------------------------------------------------------------------------------------------------
# Running a script as someone else, without a panel (for testing the administrator service's refusals and grants).
# ---------------------------------------------------------------------------------------------------------------

# Run `-Script` (PowerShell text) under another token and return Output and Code. Kind:
#   Self     this process's token, in a child
#   Filtered this administrator, unelevated (KoraTok.LaunchFiltered)
#   Account  the account in -Credential (secondary logon)
# The script's output goes to a file in -Dir (a folder the account can write: Initialize-UacWorkDir), because
# nothing can be piped back across a token boundary.
function Invoke-AsAccount([ValidateSet('Self', 'Filtered', 'Account')][string]$Kind, [string]$Script, [string]$Dir, [pscredential]$Credential, [int]$TimeoutSec = 120) {
    $id = [guid]::NewGuid().ToString('N').Substring(0, 8)
    $ps1 = Join-Path $Dir "run-$id.ps1"; $out = Join-Path $Dir "out-$id.txt"; $code = Join-Path $Dir "code-$id.txt"
    $body = "`$ErrorActionPreference = 'Continue'`r`n& {`r`n$Script`r`n} *> $(ConvertTo-PsLiteral $out)`r`n`$LASTEXITCODE | Set-Content $(ConvertTo-PsLiteral $code)`r`n"
    Set-Content -Path $ps1 -Value $body -Encoding ASCII
    $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $argText = "-NoProfile -ExecutionPolicy Bypass -File `"$ps1`""
    switch ($Kind) {
        'Self' { $procId = (Start-Process -FilePath $ps -ArgumentList $argText -PassThru -WindowStyle Hidden).Id }
        'Filtered' { $procId = [KoraTok]::LaunchFiltered($ps, "`"$ps`" $argText", $Dir) }
        'Account' {
            if (-not $Credential) { throw 'Kind Account needs -Credential' }
            $svc = Get-Service seclogon -ErrorAction SilentlyContinue
            if ($svc -and $svc.Status -ne 'Running') { Start-Service seclogon }
            $procId = (Start-Process -FilePath $ps -ArgumentList $argText -Credential $Credential -LoadUserProfile -WorkingDirectory $Dir -PassThru -WindowStyle Hidden).Id
        }
    }
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path $code) -and $sw.Elapsed.TotalSeconds -lt $TimeoutSec) { Start-Sleep -Milliseconds 300 }
    if (-not (Test-Path $code)) {
        Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue
        throw "the script run as $Kind did not finish in $TimeoutSec s"
    }
    $text = if (Test-Path $out) { (Get-Content $out -Raw) } else { '' }
    $exit = 1
    $raw = (Get-Content $code -Raw).Trim()
    if ($raw -match '^-?\d+$') { $exit = [int]$raw }
    [pscustomobject]@{ Output = "$text"; Code = $exit }
}
