//! Who may use the service. `service.json` lives in the protected config folder
//! (`%ProgramData%\Kora\service` locked to SYSTEM and Administrators; `/etc/kora`, root-owned), is
//! written by the installer (the installing administrator is on it) and changed only by an
//! administrator (`kora service allow|deny`, which asks the OS for administrator rights itself).
//!
//! The caller's identity comes from the OS connection ([`Caller`]), never from the request.

use serde::{Deserialize, Serialize};

use crate::privsvc::wire::Rejected;

pub const MAX_ACCOUNTS: usize = 64;
/// `SECURITY_MANDATORY_MEDIUM_RID`: lower-integrity (sandboxed) programs are refused.
pub const MEDIUM_INTEGRITY: u32 = 0x2000;

/// An OS account as the service stores and compares it: `uid:<n>` on Linux, the SID text on Windows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(String);

impl AccountId {
    pub fn uid(n: u32) -> AccountId {
        AccountId(format!("uid:{n}"))
    }
    /// An ordinary Windows account: see [`crate::privsvc::identity::valid_user_sid`]. Compared
    /// without case, stored upper case.
    pub fn sid(s: &str) -> Result<AccountId, Rejected> {
        let up = s.to_ascii_uppercase();
        if crate::privsvc::identity::valid_user_sid(&up) {
            Ok(AccountId(up))
        } else {
            Err(Rejected(format!("'{s}' is not an ordinary Windows account.")))
        }
    }
    /// Parse either form (the `kora service allow` argument and the file).
    pub fn parse(s: &str) -> Result<AccountId, Rejected> {
        if let Some(text) = s.strip_prefix("uid:") {
            return match text.parse::<u32>() {
                Ok(n) if n != 0 && n != u32::MAX && n.to_string() == text => Ok(AccountId::uid(n)),
                _ => Err(Rejected(format!("'{s}' is not an ordinary user account (root can always use the service)."))),
            };
        }
        AccountId::sid(s)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One allowed account. `name` is for people (`kora service status`); only `id` is compared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allowed {
    pub id: AccountId,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub version: u32,
    /// Kora version that wrote the file (for the migration step).
    #[serde(default)]
    pub written_by: String,
    pub accounts: Vec<Allowed>,
}

pub const POLICY_VERSION: u32 = 1;

impl Policy {
    pub fn new(written_by: &str) -> Policy {
        Policy { version: POLICY_VERSION, written_by: written_by.into(), accounts: vec![] }
    }

    /// Parse and validate the file's text. A file that does not parse, has an unknown version or
    /// holds an invalid / duplicate / too many accounts is an error: the service then refuses
    /// everybody (fails closed) and says why in its log.
    pub fn parse(text: &str) -> Result<Policy, String> {
        let p: Policy = serde_json::from_str(text).map_err(|e| format!("service.json is not valid: {e}"))?;
        if p.version != POLICY_VERSION {
            return Err(format!("service.json has version {}, this Kora understands {POLICY_VERSION}", p.version));
        }
        if p.accounts.len() > MAX_ACCOUNTS {
            return Err("service.json lists too many accounts".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for a in &p.accounts {
            if AccountId::parse(a.id.as_str()).map_err(|e| e.0)? != a.id || !seen.insert(a.id.clone()) {
                return Err(format!("service.json lists {} twice or in a form Kora does not write", a.id));
            }
        }
        Ok(p)
    }

    pub fn to_text(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// Add an account (no-op when present; the name is refreshed).
    pub fn allow(&mut self, id: AccountId, name: &str) -> Result<(), Rejected> {
        if let Some(a) = self.accounts.iter_mut().find(|a| a.id == id) {
            a.name = name.chars().take(96).collect();
            return Ok(());
        }
        if self.accounts.len() >= MAX_ACCOUNTS {
            return Err(Rejected("Too many accounts are allowed already.".into()));
        }
        self.accounts.push(Allowed { id, name: name.chars().take(96).collect() });
        Ok(())
    }

    pub fn deny(&mut self, id: &AccountId) -> bool {
        let n = self.accounts.len();
        self.accounts.retain(|a| &a.id != id);
        self.accounts.len() != n
    }

    pub fn contains(&self, id: &AccountId) -> bool {
        self.accounts.iter().any(|a| &a.id == id)
    }
}

/// The security descriptor of the Windows named pipe, rebuilt from the settings and set on the pipe every time the
/// service makes the next pipe instance (and the service makes one when it sees the settings change: `winsvc::watch_policy`).
/// Windows keeps ONE access list per pipe name, that of the first instance, and ignores the one given for later instances,
/// so the new instance sets it again explicitly (`SetKernelObjectSecurity`); that is how allow / deny reach the pipe's own list too:
/// owner SYSTEM (the panel checks that before it sends anything), protected (nothing inherited),
/// network logons denied, SYSTEM and Administrators full control, and the specific rights to talk
/// over the pipe (`0x120083`) for exactly the accounts on the list. (The per-connection check of [`authorise`] is the real decision; this is
/// the second fence. Unreadable settings: only SYSTEM and Administrators.)
pub fn pipe_sddl(policy: &Result<Policy, String>) -> String {
    let mut s = String::from("O:SYG:SYD:P(D;;GA;;;NU)(A;;GA;;;SY)(A;;GA;;;BA)");
    if let Ok(p) = policy {
        for a in &p.accounts {
            // only SIDs: a `uid:N` entry (a Linux file copied over) is skipped
            if crate::privsvc::identity::valid_user_sid(a.id.as_str()) {
                // read data, write data, read attributes, read control, synchronize: not FILE_APPEND_DATA
                // (= create a pipe instance), so an allowed account cannot make a look-alike pipe.
                s.push_str(&format!("(A;;0x120083;;;{})", a.id));
            }
        }
    }
    s
}

/// Who is at the other end of a connection, as the OS says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    /// `None`: the OS call failed (refused).
    pub id: Option<AccountId>,
    /// Windows: the mandatory-label RID of the client's token. `None` on Linux (not applicable)
    /// and on failure on Windows (the OS layer passes `Some(0)` there, which is refused).
    pub integrity: Option<u32>,
    /// Linux uid 0 (root can always use the service: it could do all of this anyway).
    pub is_root: bool,
    /// A remote client (Windows named pipes can be reached over SMB; the pipe rejects those, this
    /// is the second fence).
    pub remote: bool,
}

/// May this caller use the service? `policy` is `Err` when `service.json` could not be read.
pub fn authorise(caller: &Caller, policy: &Result<Policy, String>) -> Result<(), Rejected> {
    let no = |m: &str| Err(Rejected(m.to_string()));
    if caller.remote {
        return no("Kora's service does not take requests from other computers.");
    }
    if caller.is_root {
        return Ok(());
    }
    let Some(id) = &caller.id else { return no("Kora's service could not see who is asking.") };
    if let Some(i) = caller.integrity {
        if i < MEDIUM_INTEGRITY {
            return no("Kora's service does not take requests from restricted (low integrity) programs.");
        }
    }
    let Ok(policy) = policy else { return no("Kora's service settings could not be read, so it is not serving anybody. Reinstall Kora or ask an administrator.") };
    if policy.contains(id) {
        Ok(())
    } else {
        no("Kora's service is not turned on for this account. An administrator can allow it with `kora service allow`.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "S-1-5-21-1-2-3-1001";
    const OTHER: &str = "S-1-5-21-1-2-3-1002";

    fn caller(id: Option<AccountId>) -> Caller {
        Caller { id, integrity: Some(MEDIUM_INTEGRITY), is_root: false, remote: false }
    }
    fn policy() -> Result<Policy, String> {
        let mut p = Policy::new("0.8.0");
        p.allow(AccountId::sid(ME).unwrap(), "PC\\jo").unwrap();
        p.allow(AccountId::uid(1000), "jo").unwrap();
        Ok(p)
    }

    #[test]
    fn only_listed_accounts_are_served() {
        let p = policy();
        assert!(authorise(&caller(Some(AccountId::sid(ME).unwrap())), &p).is_ok());
        assert!(authorise(&caller(Some(AccountId::sid(&ME.to_lowercase()).unwrap())), &p).is_ok(), "SIDs compare without case");
        assert!(authorise(&caller(Some(AccountId::uid(1000))), &p).is_ok());
        assert!(authorise(&caller(Some(AccountId::sid(OTHER).unwrap())), &p).is_err(), "another account on the same machine");
        assert!(authorise(&caller(Some(AccountId::uid(1001))), &p).is_err());
        assert!(authorise(&caller(None), &p).is_err());
    }

    #[test]
    fn root_is_served_and_the_rest_fails_closed() {
        let root = Caller { id: Some(AccountId::uid(0)), integrity: None, is_root: true, remote: false };
        assert!(authorise(&root, &Err("broken".into())).is_ok());
        assert!(authorise(&caller(Some(AccountId::uid(1000))), &Err("broken".into())).is_err(), "unreadable settings: nobody else");
        let remote = Caller { remote: true, ..root };
        assert!(authorise(&remote, &policy()).is_err());
    }

    #[test]
    fn low_integrity_and_unknown_integrity_are_refused_on_windows() {
        let mut c = caller(Some(AccountId::sid(ME).unwrap()));
        c.integrity = Some(0x1000);
        assert!(authorise(&c, &policy()).is_err());
        c.integrity = Some(0);
        assert!(authorise(&c, &policy()).is_err());
        c.integrity = Some(0x3000);
        assert!(authorise(&c, &policy()).is_ok());
        c.integrity = None; // Linux
        assert!(authorise(&c, &policy()).is_ok());
    }

    #[test]
    fn account_ids_are_strict() {
        for bad in ["", "uid:0", "uid:-1", "uid:01", "uid:4294967295", "uid:x", "root", "S-1-5-18", "S-1-5-32-544", "S-1-1-0", "S-1-5-21-1-2-3", "S-1-5-21-1-2-3-1001-5"] {
            assert!(AccountId::parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(AccountId::parse("uid:1000").unwrap(), AccountId::uid(1000));
        assert!(AccountId::parse("S-1-12-1-1-2-3-4").is_ok(), "Entra ID account");
    }

    #[test]
    fn the_file_round_trips_and_bad_files_are_refused() {
        let p = policy().unwrap();
        assert_eq!(Policy::parse(&p.to_text()).unwrap(), p);
        assert!(Policy::parse("{}").is_err() && Policy::parse("not json").is_err());
        assert!(Policy::parse(r#"{"version":2,"accounts":[]}"#).is_err());
        assert!(Policy::parse(r#"{"version":1,"accounts":[{"id":"S-1-5-18"}]}"#).is_err(), "SYSTEM is not an ordinary account");
        let dup = format!(r#"{{"version":1,"accounts":[{{"id":"{ME}"}},{{"id":"{ME}"}}]}}"#);
        assert!(Policy::parse(&dup).is_err());
        let lower = format!(r#"{{"version":1,"accounts":[{{"id":"{}"}}]}}"#, ME.to_lowercase());
        assert!(Policy::parse(&lower).is_err(), "only the form Kora writes");
    }

    #[test]
    fn the_pipe_descriptor_lists_exactly_the_allowed_accounts() {
        let mut p = Policy::new("0.8.0");
        p.allow(AccountId::sid(ME).unwrap(), "jo").unwrap();
        p.allow(AccountId::uid(1000), "linux").unwrap();
        let s = pipe_sddl(&Ok(p));
        assert_eq!(s, format!("O:SYG:SYD:P(D;;GA;;;NU)(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x120083;;;{ME})"));
        assert_eq!(pipe_sddl(&Err("bad".into())), "O:SYG:SYD:P(D;;GA;;;NU)(A;;GA;;;SY)(A;;GA;;;BA)", "unreadable settings: administrators only");
        assert!(!pipe_sddl(&Ok(Policy::new("x"))).contains("AU") && !pipe_sddl(&Ok(Policy::new("x"))).contains(";WD)"), "never everyone");
    }

    #[test]
    fn allow_and_deny() {
        let mut p = Policy::new("0.8.0");
        let id = AccountId::uid(1000);
        p.allow(id.clone(), "jo").unwrap();
        p.allow(id.clone(), "jo2").unwrap();
        assert_eq!(p.accounts.len(), 1);
        assert_eq!(p.accounts[0].name, "jo2");
        assert!(p.deny(&id) && !p.deny(&id));
        for n in 1..=MAX_ACCOUNTS as u32 {
            p.allow(AccountId::uid(n), "").unwrap();
        }
        assert!(p.allow(AccountId::uid(9999), "").is_err());
    }
}
