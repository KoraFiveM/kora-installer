//! Administrator steps done for a process that is not elevated (the Windows panel), by an elevated
//! helper (`helper::worker`). The install's plan, its state and every file in the server folder
//! stay with the person's own process; only these steps run elevated, as
//! `kora admin-step run|undo --step <id> --dir <folder>` ([`step_args`]):
//!
//! | step              | run                                                      | undo                     |
//! |-------------------|----------------------------------------------------------|--------------------------|
//! | `prepare.db`      | install / start Kora's MariaDB, check its admin login    | -                        |
//! | `build.db`        | create the NEW database and user (Kora's MariaDB)        | drop them                |
//! | `system.firewall` | open the ports                                           | close them               |
//! | `system.service`  | program copy + Windows service + folder rights           | remove them              |
//! | `system.schedule` | the daily update task                                    | remove it                |
//!
//! The elevated child never writes into the server folder: it reads the install record, works on a
//! copy of `created.json` in memory (`Ctx::shadow`) and prints one [`StepResult`] line
//! (`helper::worker::RESULT_PREFIX`), which the person's process applies (it writes the record, the
//! database login file, ...). So files stay theirs, a failed step can be retried with a new prompt or
//! skipped, and the plan state never belongs to an administrator account.
//!
//! The database steps go to the helper only when the login is Kora's own MariaDB admin login, which
//! only administrators can read ([`db_needs_helper`]). Its password never leaves the elevated side;
//! only the new server's own database login comes back (into `database.json`, as before).

use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{check_dir_links, load_request_from, new_ctx_trusted, registered, Created, Hooks, InstallError, Result};
use crate::db::{Credentials, ProvisionState};
use crate::problem::Problem;

/// How a helper run ended (the step may have failed; `data` is its [`StepResult`]).
#[derive(Debug, Clone, PartialEq)]
pub struct AdminStepOutcome {
    pub ok: bool,
    pub data: Option<Value>,
}

/// Runs `kora admin-step <args>` with administrator rights. `Err`: it could not run at all (the
/// prompt was declined, the helper went away, ...), with what the person can do about it.
pub trait AdminRunner: Send + Sync {
    fn admin_step(&self, args: &[String], on_line: &mut dyn FnMut(&str)) -> std::result::Result<AdminStepOutcome, Problem>;
}

/// A shareable [`AdminRunner`] (printable so `Hooks` stays `Debug`).
#[derive(Clone)]
pub struct SharedAdmin(pub Arc<dyn AdminRunner>);

impl std::fmt::Debug for SharedAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedAdmin")
    }
}

/// Steps the helper runs (`admin-step run --step <id>`).
pub const RUN_STEPS: &[&str] = &["prepare.db", "build.db", "system.firewall", "system.service", "system.schedule"];
/// Steps the helper undoes (`admin-step undo --step <id>`, deleting a server).
pub const UNDO_STEPS: &[&str] = &["build.db", "system.firewall", "system.service", "system.schedule"];

/// What the helper's child hands back (one JSON line; may hold the new database login).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StepResult {
    /// The step's detail line.
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub notes: Vec<String>,
    /// The server's record after the step (written by the person's process).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<Created>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db: Option<DbResult>,
    /// The step failed: why (what was recorded before that is in `created`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Problem>,
}

/// The database part of a [`StepResult`].
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct DbResult {
    /// The new server's own login (written to `database.json`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Credentials>,
    /// What exists now in the database server (written to the state file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<ProvisionState>,
    /// An unfinished earlier attempt was cleaned up: forget its state file.
    #[serde(default)]
    pub clear_old_state: bool,
    /// Undo: the login file is no longer valid.
    #[serde(default)]
    pub remove_secret_file: bool,
}

impl std::fmt::Debug for DbResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbResult").field("state", &self.state).field("clear_old_state", &self.clear_old_state).finish_non_exhaustive()
    }
}

/// The helper's arguments for one step.
pub fn step_args(undo: bool, step: &str, dir: &Path, keep_files: bool, keep_db: bool) -> Vec<String> {
    let mut v = vec![if undo { "undo" } else { "run" }.to_string(), "--step".into(), step.into(), "--dir".into(), dir.to_string_lossy().into_owned()];
    if keep_files {
        v.push("--keep-files".into());
    }
    if keep_db {
        v.push("--keep-db".into());
    }
    v
}

/// Should the database work of an install go to the helper? Only on Windows, not elevated, with a
/// helper, when no login was given (so the login is Kora's own MariaDB one, which only
/// administrators can read).
pub fn db_needs_helper(windows: bool, admin: bool, has_helper: bool, login_given: bool) -> bool {
    windows && !admin && has_helper && !login_given
}

/// A folder the helper may work on for the person whose SID is `user_sid`: a full path, holding a
/// Kora install record, owned by that person (or, for a folder of an earlier attempt that an
/// elevated install made, by the Administrators group when the person approved as themselves).
/// `owner` = the folder's owner SID.
pub fn check_dir_for(dir: &Path, user_sid: &str, owner: Option<&str>, has_record: bool) -> std::result::Result<(), String> {
    let no = |why: &str| Err(format!("Kora's administrator helper will not work on {}: {why}.", dir.display()));
    if !dir.is_absolute() || dir.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return no("it is not a full folder path");
    }
    if !has_record {
        return no("it has no Kora install record");
    }
    match owner {
        Some(o) if !user_sid.is_empty() && o.eq_ignore_ascii_case(user_sid) => Ok(()),
        Some(_) => no("the folder does not belong to you"),
        None => no("cannot see who owns the folder"),
    }
}

/// `kora admin-step`: run (or undo) one administrator step for the server in `dir`, in THIS
/// (elevated) process, without writing into the server folder. Progress lines go to `say`.
pub fn run_admin_step(undo: bool, step: &str, dir: &Path, keep_files: bool, keep_db: bool, say: &mut dyn FnMut(&str)) -> Result<StepResult> {
    let allowed = if undo { UNDO_STEPS } else { RUN_STEPS };
    if !allowed.contains(&step) {
        return Err(InstallError(format!("'{step}' is not a step Kora's administrator helper does")));
    }
    check_dir_links(dir)?;
    let mut req = load_request_from(dir).ok_or_else(|| InstallError(format!("{} has no Kora install record", dir.display())))?;
    // Removing system-wide things: the name and ports come from the server list, not from the
    // folder's own (changeable) record, exactly as `kora delete` does.
    let trusted = if undo {
        match registered(dir) {
            Some((name, ports)) => {
                req.name = name;
                (req.ports.game, req.ports.txadmin) = (ports.game, ports.txadmin);
                true
            }
            None => false,
        }
    } else {
        true
    };
    let (tx, rx) = mpsc::channel();
    let mut ctx = new_ctx_trusted(&req, &Hooks::from_env(), true, tx, keep_db, keep_files, trusted);
    if let Some(c) = Arc::get_mut(&mut ctx) {
        c.shadow = Some(Mutex::new(super::load_created_from(dir).unwrap_or_default()));
    }
    let step = step.to_string();
    std::thread::scope(|s| {
        let h = s.spawn(move || {
            let r = super::steps::admin_part(&ctx, undo, &step);
            let created = ctx.shadow.as_ref().map(|m| m.lock().unwrap().clone());
            let notes = std::mem::take(&mut ctx.shared.lock().unwrap().notes);
            (r, created, notes)
        });
        for ev in rx {
            if !ev.detail.is_empty() {
                say(&ev.detail);
            }
        }
        let (r, created, notes) = h.join().map_err(|_| InstallError("the administrator step stopped unexpectedly".into()))?;
        Ok(match r {
            Ok((detail, db)) => StepResult { detail, notes, created, db, error: None },
            Err(e) => StepResult { detail: String::new(), notes, created, db: None, error: Some(Problem::decode(&e.0)) },
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_arguments_name_the_folder_and_options() {
        let dir = if cfg!(windows) { std::path::PathBuf::from(r"C:\Users\Jo Smith\kora\my server") } else { "/home/jo/kora/my server".into() };
        let a = step_args(false, "system.service", &dir, false, false);
        assert_eq!(a, ["run", "--step", "system.service", "--dir", &dir.to_string_lossy()]);
        let a = step_args(true, "build.db", &dir, true, true);
        assert_eq!(&a[a.len() - 2..], ["--keep-files", "--keep-db"]);
        assert_eq!(a[0], "undo");
    }

    #[test]
    fn the_database_goes_to_the_helper_only_for_kora_s_own_login_on_windows() {
        assert!(db_needs_helper(true, false, true, false));
        assert!(!db_needs_helper(false, false, true, false), "Linux: root socket / given login");
        assert!(!db_needs_helper(true, true, true, false), "already administrator");
        assert!(!db_needs_helper(true, false, false, false), "no helper: the old behaviour");
        assert!(!db_needs_helper(true, false, true, true), "a login the person typed works without administrator rights");
    }

    #[test]
    fn only_the_persons_own_folders_with_a_record_are_worked_on() {
        let dir = if cfg!(windows) { std::path::PathBuf::from(r"C:\Users\jo\kora\a") } else { "/home/jo/kora/a".into() };
        let me = "S-1-5-21-1-2-3-1001";
        assert!(check_dir_for(&dir, me, Some(me), true).is_ok());
        assert!(check_dir_for(&dir, me, Some("s-1-5-21-1-2-3-1001"), true).is_ok(), "SIDs compare without case");
        assert!(check_dir_for(&dir, me, Some("S-1-5-18"), true).unwrap_err().contains("does not belong to you"));
        assert!(check_dir_for(&dir, me, Some("S-1-5-32-544"), true).is_err(), "a folder administrators own is not the person's");
        assert!(check_dir_for(&dir, me, None, true).is_err());
        assert!(check_dir_for(&dir, me, Some(me), false).unwrap_err().contains("no Kora install record"));
        assert!(check_dir_for(Path::new("relative"), me, Some(me), true).is_err());
        assert!(check_dir_for(&dir, "", Some(""), true).is_err(), "no known user: refused");
    }

    #[test]
    fn unknown_steps_are_refused_before_anything_is_read() {
        let e = run_admin_step(false, "build.config", Path::new("/nonexistent"), false, false, &mut |_| {}).unwrap_err();
        assert!(e.0.contains("not a step"), "{e:?}");
        assert!(run_admin_step(true, "prepare.db", Path::new("/nonexistent"), false, false, &mut |_| {}).is_err());
    }

    #[test]
    fn a_step_result_never_prints_the_database_password() {
        let r = DbResult {
            credentials: Some(Credentials { host: "127.0.0.1".into(), port: 3306, database: "kora_x".into(), user: "kora_x".into(), password: crate::db::Secret::new("TOPSECRET") }),
            ..Default::default()
        };
        assert!(!format!("{r:?}").contains("TOPSECRET"));
        let s = StepResult { db: Some(r), ..Default::default() };
        assert!(!format!("{s:?}").contains("TOPSECRET"));
        // ...but it does travel to the person's process (it goes into database.json).
        let back: StepResult = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        assert_eq!(back.db.unwrap().credentials.unwrap().password.expose(), "TOPSECRET");
    }
}
