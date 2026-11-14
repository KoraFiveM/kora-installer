//! The real [`Sys`]: files, `reg`/PowerShell on Windows, symlinks on Unix.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{lnk, AppError, Result, ShortcutSpec, Sys, UninstallEntry, UNINSTALL_KEY};

pub struct RealSys;

impl RealSys {
    pub fn new() -> Self {
        RealSys
    }
}

impl Default for RealSys {
    fn default() -> Self {
        Self::new()
    }
}

fn io<T>(r: std::io::Result<T>, what: impl AsRef<str>) -> Result<T> {
    r.map_err(|e| AppError(format!("{}: {e}", what.as_ref())))
}

/// Run a PowerShell script. Values travel in environment variables (never spliced into the script), so
/// paths with spaces, quotes or non-ASCII characters need no escaping.
fn powershell(script: &str, env: &[(&str, OsString)]) -> Result<String> {
    let mut c = Command::new("powershell.exe");
    let script = format!("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; {script}");
    c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script]);
    for (k, v) in env {
        c.env(k, v);
    }
    let out = io(c.output(), "cannot run PowerShell")?;
    if !out.status.success() {
        let e = String::from_utf8_lossy(&out.stderr);
        let o = String::from_utf8_lossy(&out.stdout);
        return Err(AppError(format!("PowerShell failed: {}{}", e.trim(), o.trim())));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_start_matches('\u{feff}').to_string())
}

const ENV_KEY: &str = r"HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

fn hklm_uninstall() -> String {
    format!(r"HKLM:\{UNINSTALL_KEY}")
}

impl Sys for RealSys {
    fn exists(&self, p: &Path) -> bool {
        p.symlink_metadata().is_ok()
    }

    fn read(&self, p: &Path) -> Option<Vec<u8>> {
        std::fs::read(p).ok()
    }

    fn sha256(&self, p: &Path) -> Option<String> {
        crate::service::binary::file_sha256(p).ok()
    }

    fn read_link(&self, p: &Path) -> Option<PathBuf> {
        std::fs::read_link(p).ok()
    }

    fn write(&mut self, p: &Path, bytes: &[u8], mode: u32) -> Result<()> {
        if let Some(d) = p.parent() {
            io(std::fs::create_dir_all(d), format!("cannot create {}", d.display()))?;
        }
        let tmp = p.with_extension("kora-tmp");
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = io(std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(mode).open(&tmp), format!("cannot write {}", p.display()))?;
            io(f.write_all(bytes), format!("cannot write {}", p.display()))?;
        }
        #[cfg(not(unix))]
        {
            let _ = mode;
            io(std::fs::write(&tmp, bytes), format!("cannot write {}", p.display()))?;
        }
        if cfg!(windows) && p.exists() {
            let _ = std::fs::remove_file(p);
        }
        io(std::fs::rename(&tmp, p), format!("cannot write {}", p.display()))
    }

    fn copy_exe(&mut self, from: &Path, to: &Path) -> Result<()> {
        let dir = to.parent().ok_or_else(|| AppError("no folder to copy into".into()))?;
        io(std::fs::create_dir_all(dir), format!("cannot create {}", dir.display()))?;
        sweep_leftovers(dir);
        let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let tmp = dir.join(format!("{name}.new"));
        io(std::fs::copy(from, &tmp), format!("cannot copy {} to {}", from.display(), tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            io(std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)), "cannot make the copy executable")?;
        }
        if cfg!(windows) && to.exists() {
            // A running program cannot be overwritten but it can be renamed: move it aside, put the new one
            // in place, then delete the old one (now, or at the next start if it is still in use).
            let old = dir.join(format!("{name}.old-{}", std::process::id()));
            if let Err(e) = std::fs::rename(to, &old) {
                let _ = std::fs::remove_file(&tmp);
                return Err(AppError(format!("cannot replace {} (is Kora running?): {e}", to.display())));
            }
            if let Err(e) = std::fs::rename(&tmp, to) {
                let _ = std::fs::rename(&old, to);
                let _ = std::fs::remove_file(&tmp);
                return Err(AppError(format!("cannot put the new program in place: {e}")));
            }
            if std::fs::remove_file(&old).is_err() {
                schedule_delete(&old);
            }
            return Ok(());
        }
        io(std::fs::rename(&tmp, to), format!("cannot put the program at {}", to.display()))
    }

    fn symlink(&mut self, target: &Path, link: &Path) -> Result<()> {
        if let Some(d) = link.parent() {
            io(std::fs::create_dir_all(d), format!("cannot create {}", d.display()))?;
        }
        #[cfg(unix)]
        {
            io(std::os::unix::fs::symlink(target, link), format!("cannot link {}", link.display()))
        }
        #[cfg(not(unix))]
        {
            let _ = target;
            Err(AppError("links are only used on Linux/macOS".into()))
        }
    }

    fn remove_file(&mut self, p: &Path) -> Result<()> {
        match std::fs::remove_file(p) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) if cfg!(windows) => {
                // In use (for example the running uninstaller): rename away, delete later.
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let old = p.with_file_name(format!("{name}.old-{}", std::process::id()));
                if std::fs::rename(p, &old).is_ok() {
                    if std::fs::remove_file(&old).is_err() {
                        schedule_delete(&old);
                    }
                    Ok(())
                } else {
                    Err(AppError(format!("cannot delete {}: {e}", p.display())))
                }
            }
            Err(e) => Err(AppError(format!("cannot delete {}: {e}", p.display()))),
        }
    }

    fn remove_dir_if_empty(&mut self, p: &Path) -> Result<()> {
        if cfg!(windows) {
            sweep_leftovers(p);
        }
        match std::fs::remove_dir(p) {
            Ok(()) => Ok(()),
            // Not empty (service copies, other files) or already gone: both are fine.
            Err(_) => Ok(()),
        }
    }

    fn machine_path(&self) -> Result<Option<String>> {
        if !cfg!(windows) {
            return Ok(None);
        }
        let script = format!(
            "$k = Get-Item -LiteralPath '{ENV_KEY}'; [Console]::Out.Write($k.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames))"
        );
        let out = powershell(&script, &[])?;
        Ok(Some(out))
    }

    fn set_machine_path(&mut self, value: &str) -> Result<()> {
        let script = format!("Set-ItemProperty -LiteralPath '{ENV_KEY}' -Name Path -Type ExpandString -Value $env:KORA_VALUE");
        powershell(&script, &[("KORA_VALUE", value.into())]).map(|_| ())
    }

    fn broadcast_env_change(&mut self) {
        if !cfg!(windows) {
            return;
        }
        let script = "Add-Type -Namespace Kora -Name Native -MemberDefinition '[DllImport(\"user32.dll\", SetLastError=true, CharSet=CharSet.Auto)] public static extern IntPtr SendMessageTimeout(IntPtr h, uint m, UIntPtr w, string l, uint f, uint t, out UIntPtr r);'; \
            $r = [UIntPtr]::Zero; [void][Kora.Native]::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$r)";
        let _ = powershell(script, &[]); // best effort: a new terminal picks the PATH up anyway
    }

    fn create_shortcut(&mut self, s: &ShortcutSpec) -> Result<()> {
        if let Some(d) = s.path.parent() {
            io(std::fs::create_dir_all(d), format!("cannot create {}", d.display()))?;
        }
        let script = "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:KORA_LNK); \
            $s.TargetPath = $env:KORA_TARGET; $s.Arguments = $env:KORA_ARGS; $s.WorkingDirectory = $env:KORA_WORKDIR; \
            $s.IconLocation = $env:KORA_ICON; $s.Description = $env:KORA_DESC; $s.Save()";
        powershell(
            script,
            &[
                ("KORA_LNK", s.path.clone().into_os_string()),
                ("KORA_TARGET", s.target.clone().into_os_string()),
                ("KORA_ARGS", s.args.clone().into()),
                ("KORA_WORKDIR", s.workdir.clone().into_os_string()),
                ("KORA_ICON", s.icon.clone().into()),
                ("KORA_DESC", s.description.clone().into()),
            ],
        )?;
        if s.run_as_admin {
            let bytes = io(std::fs::read(&s.path), "cannot read the new shortcut")?;
            let patched = lnk::set_run_as_admin(&bytes, true).map_err(AppError)?;
            io(std::fs::write(&s.path, patched), "cannot mark the shortcut 'Run as administrator'")?;
        }
        Ok(())
    }

    fn read_shortcut(&self, p: &Path) -> Option<ShortcutSpec> {
        if !cfg!(windows) || !p.is_file() {
            return None;
        }
        let script = "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:KORA_LNK); \
            [Console]::Out.Write(($s.TargetPath, $s.Arguments, $s.WorkingDirectory, $s.IconLocation, $s.Description) -join [char]31)";
        let out = powershell(script, &[("KORA_LNK", p.as_os_str().to_os_string())]).ok()?;
        let f: Vec<&str> = out.split('\u{1f}').collect();
        if f.len() != 5 {
            return None;
        }
        let admin = lnk::run_as_admin(&std::fs::read(p).ok()?)?;
        Some(ShortcutSpec {
            path: p.to_path_buf(),
            target: PathBuf::from(f[0]),
            args: f[1].to_string(),
            workdir: PathBuf::from(f[2]),
            icon: f[3].to_string(),
            description: f[4].to_string(),
            run_as_admin: admin,
        })
    }

    fn uninstall_entry(&self) -> Option<UninstallEntry> {
        if !cfg!(windows) {
            return None;
        }
        let key = hklm_uninstall();
        let script = format!(
            "$k = Get-Item -LiteralPath '{key}' -ErrorAction SilentlyContinue; if ($k) {{ [Console]::Out.Write((@('DisplayName','DisplayVersion','Publisher','InstallLocation','UninstallString','DisplayIcon') | ForEach-Object {{ [string]$k.GetValue($_, '') }}) -join [char]31) }}"
        );
        let out = powershell(&script, &[]).ok()?;
        let f: Vec<&str> = out.split('\u{1f}').collect();
        (f.len() == 6).then(|| UninstallEntry {
            display_name: f[0].into(),
            display_version: f[1].into(),
            publisher: f[2].into(),
            install_location: f[3].into(),
            uninstall_string: f[4].into(),
            display_icon: f[5].into(),
        })
    }

    fn set_uninstall_entry(&mut self, e: &UninstallEntry) -> Result<()> {
        let key = hklm_uninstall();
        let script = format!(
            "$p = '{key}'; New-Item -Path $p -Force | Out-Null; \
             Set-ItemProperty -LiteralPath $p -Name DisplayName -Value $env:KORA_NAME -Type String; \
             Set-ItemProperty -LiteralPath $p -Name DisplayVersion -Value $env:KORA_VERSION -Type String; \
             Set-ItemProperty -LiteralPath $p -Name Publisher -Value $env:KORA_PUBLISHER -Type String; \
             Set-ItemProperty -LiteralPath $p -Name InstallLocation -Value $env:KORA_LOCATION -Type String; \
             Set-ItemProperty -LiteralPath $p -Name UninstallString -Value $env:KORA_UNINSTALL -Type String; \
             Set-ItemProperty -LiteralPath $p -Name DisplayIcon -Value $env:KORA_ICON -Type String; \
             Set-ItemProperty -LiteralPath $p -Name NoModify -Value 1 -Type DWord; \
             Set-ItemProperty -LiteralPath $p -Name NoRepair -Value 1 -Type DWord"
        );
        powershell(
            &script,
            &[
                ("KORA_NAME", e.display_name.clone().into()),
                ("KORA_VERSION", e.display_version.clone().into()),
                ("KORA_PUBLISHER", e.publisher.clone().into()),
                ("KORA_LOCATION", e.install_location.clone().into()),
                ("KORA_UNINSTALL", e.uninstall_string.clone().into()),
                ("KORA_ICON", e.display_icon.clone().into()),
            ],
        )
        .map(|_| ())
    }

    fn remove_uninstall_entry(&mut self) -> Result<()> {
        let key = hklm_uninstall();
        powershell(&format!("Remove-Item -LiteralPath '{key}' -Recurse -Force -ErrorAction SilentlyContinue"), &[]).map(|_| ())
    }
}

/// Windows: delete leftovers of earlier replacements (`kora.exe.old-*`, `kora.exe.new`).
fn sweep_leftovers(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.starts_with("kora.exe.old-") || n.starts_with("kora.old-") || n.ends_with(".kora-tmp") || n == "kora.exe.new" || n == "kora.new" {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Windows: a file that is still in use is deleted at the next sign-in (RunOnce).
fn schedule_delete(p: &Path) {
    if !cfg!(windows) {
        return;
    }
    let script = r#"New-ItemProperty -Path 'HKLM:\Software\Microsoft\Windows\CurrentVersion\RunOnce' -Name ('KoraCleanup' + [guid]::NewGuid().ToString('N')) -Value ('cmd.exe /c del /f /q "' + $env:KORA_FILE + '"') -PropertyType String -Force | Out-Null"#;
    let _ = powershell(script, &[("KORA_FILE", p.as_os_str().to_os_string())]);
}
