//! The Kora privileged service: ONE small system service per machine (Windows: service `KoraService`;
//! Linux: systemd unit `kora.service`) that does a fixed list of administrator operations for the
//! unprivileged per-user panel, with no prompt, after one administrator approval at install time.
//!
//! This module is the DESIGN and the pure decision code (everything here is tested on every OS; the
//! thin OS layers, the socket or named pipe and the process spawning, call into it). It replaces
//! three mechanisms: the optional Linux helper (`helper::server`), the Windows UAC worker
//! (`helper::worker`, `helper::winpipe`) and the inert persistent-helper groundwork
//! (`helper::winhelper`). Until the slices that wire it in have landed those still work; the plan is
//! in `docs/privileged-service.md`.
//!
//! # Shape
//!
//! * **Program.** Installed by the MSI / the `.deb` (or the root-owned tarball install) into a
//!   protected folder (`%ProgramFiles%\Kora`, `/usr/lib/kora`) that only administrators / root can
//!   write. The service runs that program, never a copy in a user-writable place, and refuses to
//!   start when its own file or any parent folder is writable by someone else ([`paths::trusted_owner`]).
//! * **Who may ask.** [`policy::Policy`] (`service.json` in the protected config folder, written by
//!   the installer from what the installing administrator chose) lists the accounts allowed to use the
//!   service. The OS identity of the caller is read from the connection itself on EVERY connection
//!   (Windows: the client's token on the named pipe, SID; Linux: `SO_PEERCRED` on the Unix socket,
//!   uid) and checked by [`policy::authorise`]. Nothing the caller sends can name its own identity.
//! * **What may be asked.** [`op::Op`] is the whole allow-list: a typed value parsed by [`op::parse`]
//!   from the wire request (`verb` + `args`, the framing of `helper::Request`). The parser is strict
//!   (exact flags, exact value shapes); anything else is refused. The service then does not forward
//!   the caller's argument text anywhere: it re-creates the arguments from the typed value
//!   ([`op::Op::child_args`]), so whatever the parser did not understand cannot travel on.
//! * **One at a time, with time limits.** One privileged operation runs at a time across all
//!   connections; each has a time limit ([`op::Op::time_limit`]); a client that goes away cancels
//!   the running operation.
//! * **Clean child.** Operations run as a child of the protected program with an environment built
//!   from nothing ([`paths::child_env`]): system-only `PATH`, no `KORA_*` override variable, no
//!   `GIT_*`, `HOME`/`USERPROFILE` re-derived by the service from the caller's account (never from
//!   the request).
//! * **Audit.** Every operation, allowed or refused, appends one line to the service's audit log
//!   ([`audit::line`]): when, which account, what (the operation's name and the server's short
//!   name), the outcome. Never a path, an argument value or a secret.
//!
//! # Threat model
//!
//! The strongest attacker we design against: **any program running as an allowed user**, for
//! example malware the person ran, or the game server's own service account for an account that is
//! allowed. It can open the service's socket or pipe at any time, without a prompt, and send
//! anything. There is no secret between it and the panel: the panel is the same user. So:
//!
//! | attack | closed by |
//! |---|---|
//! | the service runs a program the user can replace (the old UAC worker elevates `%LOCALAPPDATA%\Kora\bin\kora.exe`) | the service and every child run only the protected program; [`paths::trusted_owner`]; nothing is looked up through `PATH` ([`paths::child_env`]); no DLL / plug-in folder the user can write |
//! | a request names a path, a name or a file the caller controls, and the service trusts it | the service **re-derives** instead: the server folder is checked on the privileged side without following links, owned by the caller ([`ownership::folder_ok`]) and must hold a Kora install record; names and ports that matter for undoing come from the server list, not from the folder's own record; steps never write inside a user folder |
//! | one user removes or changes another user's server, service, task, firewall rule or database | the ownership ledger (`owners.json`, protected, keys `service:`, `firewall:`, `task:`, `db:`) records the owning account for everything the service made; making needs "not owned by someone else", removing needs "owned by the caller" ([`ownership::allow`]); an unrecorded item (made by an older Kora) is changed only after an administrator approves at the computer |
//! | open dangerous firewall ports | [`op::port_ok`]: at least 1024, never remote administration or database ports |
//! | register a service that runs attacker code | the service unit / Windows service always runs the protected program with fixed arguments `service-host <slug>`; the program re-reads the server folder as the server's own account; the privileges are ordinary-user ones ([`op::Op::Step`] `system.service`) |
//! | environment tricks (`KORA_MARIADB_HOME`, `KORA_SELF_UPDATE_API`, `GIT_*`, `LD_PRELOAD`, user `PATH`) | clean child environment; the request accepts only the two secret names ([`op::SECRET_ENV`]) |
//! | `self-update` to a file the caller chose, or a downgrade | the service downloads the release itself, verifies the release checksum (and the provenance attestation when available), and refuses an older version ([`op::Op::UpdateApply`]) |
//! | oversized or malformed requests, floods | size caps ([`op::MAX_LINE`]), connection cap, per-operation time limit, one privileged operation at a time |
//! | spoofing the service so the panel hands it a licence key or database password | Windows: the panel checks the pipe is owned by SYSTEM before sending anything (`winhelper::check_server_owner`); Linux: the socket lives in a root-owned folder under `/run`, and the panel checks the socket's owner is root ([`paths::socket_owner_ok`]) |
//! | a low-integrity sandboxed program of an allowed user | Windows: refused unless integrity is medium or higher ([`policy::authorise`]) |
//! | secrets | licence key and database password only travel in the request's `env` (never `args`), are masked in output, never logged ([`audit`]) |
//!
//! What this does NOT defend against, and the documentation says so (SECURITY.md): an allowed user
//! can do the allow-listed things without a prompt (open a firewall port >= 1024 for their server,
//! register their own server's service, create their own databases, start/stop Kora's database,
//! apply an official Kora update). Each is limited to that user's own items. An administrator who
//! does not want a user to have this removes the account from the list (`kora service allow` /
//! `deny`, or uninstalls the service).

pub mod audit;
pub mod dispatch;
pub mod install;
pub mod manage;
pub mod op;
pub mod ownership;
pub mod paths;
pub mod policy;
pub mod problems;
#[cfg(unix)]
pub mod unixsvc;
