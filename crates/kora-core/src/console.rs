//! Live console: what the manager's Console screen reads and writes.
//!
//! **Output.** `kora service-host` copies everything FXServer prints to `<server>/.kora/console.log`.
//! A server started detached writes `.kora/server.log` instead. [`source`] picks the newest of them.
//!
//! **Input.** FXServer under txAdmin does not take commands on its standard input (txAdmin owns the
//! real server process), and txAdmin's own console API needs an admin login Kora does not have. So
//! the manager appends a line to `<server>/.kora/console.in`, and a tiny server-side resource,
//! [`RESOURCE`] (`kora_console`), polls that file and runs each new line with `ExecuteCommand`. The
//! resource is added to a server by [`install`] (idempotent) and takes effect on the next start.
//! Only someone who can already write to the server folder can queue a command.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Name of the helper resource.
pub const RESOURCE: &str = "kora_console";
/// Longest command accepted.
pub const MAX_COMMAND: usize = 1000;

const FXMANIFEST: &str = "fx_version 'cerulean'\ngame 'gta5'\nauthor 'Kora'\ndescription 'Runs commands typed into the Kora manager console'\nserver_script 'server.lua'\n";

const SERVER_LUA: &str = r#"-- Kora live console: runs the lines the Kora manager appends to the command file.
local file = GetConvar('kora_console_file', '')
if file == '' then return end

local offset = 0
do -- skip anything queued while the server was off
  local f = io.open(file, 'rb')
  if f then offset = f:seek('end'); f:close() end
end

CreateThread(function()
  while true do
    Wait(500)
    local f = io.open(file, 'rb')
    if f then
      local size = f:seek('end')
      if size < offset then offset = 0 end
      if size > offset then
        f:seek('set', offset)
        local data = f:read(size - offset) or ''
        local last = data:match('.*()\n')
        if last then
          offset = offset + last
          for line in data:sub(1, last):gmatch('[^\r\n]+') do
            print('> ' .. line)
            ExecuteCommand(line)
          end
        end
      end
      f:close()
    end
  end
end)
"#;

pub fn input_path(dir: &Path) -> PathBuf {
    dir.join(".kora").join("console.in")
}

/// The log to show: the most recently written of the console / server / service-host logs.
pub fn source(dir: &Path) -> Option<PathBuf> {
    ["console.log", "server.log", "service-host.log"]
        .iter()
        .map(|n| dir.join(".kora").join(n))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

/// Make `text` safe to queue: no control characters or line breaks, trimmed, not empty, not too long.
pub fn clean_command(text: &str) -> Result<String, String> {
    let c: String = text.chars().filter(|c| !c.is_control()).collect();
    let c = c.trim().to_string();
    if c.is_empty() {
        return Err("Type a command first.".into());
    }
    if c.chars().count() > MAX_COMMAND {
        return Err(format!("That command is too long (the limit is {MAX_COMMAND} characters)."));
    }
    Ok(c)
}

/// Queue one command for the running server.
pub fn send(dir: &Path, text: &str) -> Result<(), String> {
    let line = clean_command(text)?;
    if !is_installed(dir) {
        return Err("This server does not have the console helper yet. Turn on the live console first (it needs a restart).".into());
    }
    let p = input_path(dir);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("Can't write {}: {e}", d.display()))?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&p).map_err(|e| format!("Can't write {}: {e}", p.display()))?;
    writeln!(f, "{line}").map_err(|e| format!("Can't write {}: {e}", p.display()))
}

fn resource_dir(dir: &Path) -> PathBuf {
    dir.join("resources").join("[kora-tools]").join(RESOURCE)
}

fn cfg_lines(dir: &Path) -> String {
    // Forward slashes work for FXServer on Windows too and need no escaping inside the quotes.
    let file = input_path(dir).to_string_lossy().replace('\\', "/");
    format!("\n# Kora live console (added by Kora)\nset kora_console_file \"{file}\"\nadd_ace resource.{RESOURCE} command allow\nensure {RESOURCE}\n")
}

/// Is the helper resource in place and ensured by `server.cfg`?
pub fn is_installed(dir: &Path) -> bool {
    resource_dir(dir).join("server.lua").is_file()
        && std::fs::read_to_string(dir.join("server.cfg")).is_ok_and(|c| c.lines().any(|l| l.trim() == format!("ensure {RESOURCE}")))
}

/// Add the helper resource and its `server.cfg` lines. `Ok(true)` = something changed (the server needs
/// a restart), `Ok(false)` = it was already there.
pub fn install(dir: &Path) -> Result<bool, String> {
    if is_installed(dir) {
        return Ok(false);
    }
    let r = resource_dir(dir);
    std::fs::create_dir_all(&r).map_err(|e| format!("Can't create {}: {e}", r.display()))?;
    std::fs::write(r.join("fxmanifest.lua"), FXMANIFEST).map_err(|e| format!("Can't write the console resource: {e}"))?;
    std::fs::write(r.join("server.lua"), SERVER_LUA).map_err(|e| format!("Can't write the console resource: {e}"))?;
    let cfg_path = dir.join("server.cfg");
    let mut cfg = std::fs::read_to_string(&cfg_path).map_err(|e| format!("Can't read {}: {e}", cfg_path.display()))?;
    if !cfg.contains("set kora_console_file") {
        cfg.push_str(&cfg_lines(dir));
    } else if !cfg.lines().any(|l| l.trim() == format!("ensure {RESOURCE}")) {
        cfg.push_str(&format!("ensure {RESOURCE}\n"));
    }
    std::fs::write(&cfg_path, cfg).map_err(|e| format!("Can't write {}: {e}", cfg_path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("server.cfg"), "exec secrets.cfg\nensure kora_core\n").unwrap();
        d
    }

    #[test]
    fn commands_are_cleaned_and_limited() {
        assert_eq!(clean_command("  say hi\u{1b}[0m\n").unwrap(), "say hi[0m");
        assert!(clean_command("  \n").is_err());
        assert!(clean_command(&"x".repeat(MAX_COMMAND + 1)).is_err());
        assert_eq!(clean_command("a\r\nb").unwrap(), "ab", "a line break can never smuggle in a second command");
    }

    #[test]
    fn install_is_idempotent_and_edits_the_cfg_once() {
        let d = server();
        assert!(!is_installed(d.path()));
        assert!(install(d.path()).unwrap());
        assert!(is_installed(d.path()));
        assert!(!install(d.path()).unwrap());
        let cfg = std::fs::read_to_string(d.path().join("server.cfg")).unwrap();
        assert_eq!(cfg.matches("ensure kora_console").count(), 1);
        assert_eq!(cfg.matches("set kora_console_file").count(), 1);
        assert!(cfg.contains("add_ace resource.kora_console command allow"));
        assert!(cfg.starts_with("exec secrets.cfg\nensure kora_core\n"), "existing lines are untouched");
    }

    #[test]
    fn send_needs_the_helper_then_appends_lines() {
        let d = server();
        assert!(send(d.path(), "status").unwrap_err().contains("live console"));
        install(d.path()).unwrap();
        send(d.path(), "say hello").unwrap();
        send(d.path(), "  restart kora_core ").unwrap();
        assert_eq!(std::fs::read_to_string(input_path(d.path())).unwrap(), "say hello\nrestart kora_core\n");
    }

    #[test]
    fn source_prefers_the_newest_log() {
        let d = server();
        assert!(source(d.path()).is_none());
        let k = d.path().join(".kora");
        std::fs::create_dir_all(&k).unwrap();
        std::fs::write(k.join("server.log"), "old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(k.join("console.log"), "new").unwrap();
        assert!(source(d.path()).unwrap().ends_with("console.log"));
    }
}
