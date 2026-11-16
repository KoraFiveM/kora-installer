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
//!
//! **Loaded yet?** [`install`] on a running server changes nothing until the next restart, and the
//! helper drops what was queued before it started. So [`install`] leaves a note with a random token
//! (`.kora/console.in.pending`); the helper prints the token and deletes the note when it starts.
//! While the note is there and the server's log shows no token, [`waiting_for_restart`] is true.
//! It is false whenever that cannot be told (no note, no log written since, an old note), so a
//! server running an older helper, which knows nothing of the note, is never refused.

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

do -- "Enable console" in Kora leaves a note; answering it tells Kora this helper is loaded
  local p = io.open(file .. '.pending', 'rb')
  if p then
    local token = (p:read('*l') or ''):match('^%x+')
    p:close()
    if token then print('Kora live console ready ' .. token) end
    pcall(os.remove, file .. '.pending')
  end
end

local offset = 0
do -- drop anything queued while the server was off: empty the file, or skip past it if that fails
  local f = io.open(file, 'rb')
  if f then
    f:close()
    local ok, w = pcall(io.open, file, 'w')
    if ok and w then w:close() end
    f = io.open(file, 'rb')
    if f then offset = f:seek('end'); f:close() end
  end
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

/// What to say while the helper is on disk but not loaded by the running server.
pub const NEEDS_RESTART: &str = "The live console starts working after the server's next restart. Restart the server, then try again.";
/// What the helper prints, followed by the token of the note, when it starts.
const READY: &str = "Kora live console ready ";
/// A note older than this is ignored (and removed): nobody is refused forever.
const PENDING_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

pub fn input_path(dir: &Path) -> PathBuf {
    dir.join(".kora").join("console.in")
}

fn pending_path(dir: &Path) -> PathBuf {
    dir.join(".kora").join("console.in.pending")
}

fn log_has(path: &Path, needle: &[u8]) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut buf = vec![0u8; 64 * 1024 + needle.len()];
    let mut kept = 0;
    loop {
        let end = match f.read(&mut buf[kept..]) {
            Ok(0) | Err(_) => return false,
            Ok(n) => kept + n,
        };
        if buf[..end].windows(needle.len()).any(|w| w == needle) {
            return true;
        }
        // Keep the tail: the text may sit across two reads.
        kept = end.min(needle.len() - 1);
        buf.copy_within(end - kept..end, 0);
    }
}

/// Was the helper added (or changed) while the server ran, and the server not restarted since?
/// Then commands would be queued and never run. False when it cannot be told (see the module text).
pub fn waiting_for_restart(dir: &Path) -> bool {
    use std::io::Read;
    let p = pending_path(dir);
    let Ok(noted) = std::fs::metadata(&p).and_then(|m| m.modified()) else { return false };
    let mut token = String::new();
    let _ = std::fs::File::open(&p).map(|f| f.take(64).read_to_string(&mut token));
    let token = token.trim();
    let forget = || {
        let _ = std::fs::remove_file(&p);
        false
    };
    if token.is_empty() || !token.bytes().all(|b| b.is_ascii_hexdigit()) || noted.elapsed().is_ok_and(|age| age > PENDING_MAX_AGE) {
        return forget();
    }
    let needle = format!("{READY}{token}");
    let mut written_since = false;
    for name in ["console.log", "console.log.old", "server.log", "service-host.log"] {
        let log = dir.join(".kora").join(name);
        let Ok(changed) = std::fs::metadata(&log).and_then(|m| m.modified()) else { continue };
        if log_has(&log, needle.as_bytes()) {
            return forget();
        }
        written_since |= changed > noted;
    }
    // A log that has not moved since the note is not this server's output: nothing can be told.
    written_since
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
    if waiting_for_restart(dir) {
        return Err(NEEDS_RESTART.into());
    }
    let p = input_path(dir);
    let mut f = crate::fsutil::open_no_follow(&p, crate::fsutil::Access::Append, Some(0o644)).map_err(|e| format!("Can't write {}: {e}", p.display()))?;
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
    // Root runs this during an install: never through a link planted in the server folder.
    let r = resource_dir(dir);
    let put = |name: &str, text: &str| crate::fsutil::write_no_follow(&r.join(name), text.as_bytes(), 0o644).map_err(|e| format!("Can't write the console resource: {e}"));
    put("fxmanifest.lua", FXMANIFEST)?;
    put("server.lua", SERVER_LUA)?;
    let cfg_path = dir.join("server.cfg");
    let mut cfg = std::fs::read_to_string(&cfg_path).map_err(|e| format!("Can't read {}: {e}", cfg_path.display()))?;
    if !cfg.contains("set kora_console_file") {
        cfg.push_str(&cfg_lines(dir));
    } else if !cfg.lines().any(|l| l.trim() == format!("ensure {RESOURCE}")) {
        cfg.push_str(&format!("ensure {RESOURCE}\n"));
    }
    crate::fsutil::write_no_follow(&cfg_path, cfg.as_bytes(), 0o644).map_err(|e| format!("Can't write {}: {e}", cfg_path.display()))?;
    // The note the helper answers at its next start (see `waiting_for_restart`). Optional: without it nothing is refused.
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let _ = crate::fsutil::write_no_follow(&pending_path(dir), format!("{:x}\n", nanos ^ (u128::from(std::process::id()) << 64)).as_bytes(), 0o644);
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
    fn a_helper_added_while_the_server_runs_waits_for_the_restart() {
        let d = server();
        assert!(!waiting_for_restart(d.path()), "no helper, no note");
        install(d.path()).unwrap();
        let note = pending_path(d.path());
        let token = std::fs::read_to_string(&note).unwrap().trim().to_string();
        assert!(!token.is_empty() && token.bytes().all(|b| b.is_ascii_hexdigit()), "{token}");
        assert!(SERVER_LUA.contains(&format!("print('{READY}' .. token)")) && SERVER_LUA.contains("file .. '.pending'"), "the helper answers the note");
        assert!(!waiting_for_restart(d.path()), "no log written since: cannot tell, so nothing is refused");
        // The running server keeps printing, and never says the helper started.
        let log = d.path().join(".kora").join("console.log");
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(&log, "[script:chat] hello\n").unwrap();
        assert!(waiting_for_restart(d.path()));
        assert_eq!(send(d.path(), "status").unwrap_err(), NEEDS_RESTART);
        assert!(!input_path(d.path()).exists(), "nothing is queued for a helper that would drop it");
        // After the restart the helper printed the token (here it could not delete the note itself).
        std::fs::write(&log, format!("{}\n[script:kora_console] {READY}{token}\n", "x".repeat(70_000))).unwrap();
        assert!(!waiting_for_restart(d.path()));
        assert!(!note.exists(), "the note is gone once answered");
        send(d.path(), "status").unwrap();
        // A server whose helper was installed by an older Kora has no note and is never refused.
        std::fs::write(&log, "more output\n").unwrap();
        assert!(!waiting_for_restart(d.path()));
    }

    #[test]
    fn a_broken_note_is_dropped_instead_of_refusing_forever() {
        let d = server();
        install(d.path()).unwrap();
        let note = pending_path(d.path());
        std::fs::write(&note, "not a token\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(d.path().join(".kora").join("server.log"), "output\n").unwrap();
        assert!(!waiting_for_restart(d.path()));
        assert!(!note.exists());
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
