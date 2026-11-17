# The Kora privileged service (design and plan)

Status: **slice 1 of 7 is in the tree** (the design as tested, pure code in `crates/kora-core/src/privsvc/`).
Nothing in the panel or the CLI uses it yet; the Linux helper and the Windows UAC worker still work as in v0.7.0.
The threat model is in the module documentation of `privsvc` (`crates/kora-core/src/privsvc/mod.rs`) and, once
the service is live, in SECURITY.md.

## What and why

Kora gets administrator rights three ways today: an optional Linux helper, a Windows worker started through a
UAC prompt that elevates a `kora.exe` in a user-writable folder, and elevated terminals. The security review of
v0.7.0 found the second the weakest point (elevating a program the user can replace, driven over a pipe by an
unprivileged process), and remote installs on Windows with nobody at the desktop cannot work with a prompt.

Target: **one small service** per machine, installed by a proper installer into a protected folder, doing a fixed
allow-list of validated operations for the unprivileged per-user panel.

| | Windows | Linux |
|---|---|---|
| installer | MSI (WiX, built in CI): Program Files, service, PATH, shortcuts, Apps & features | `.deb` (and a root-owned tarball install elsewhere): `/usr/lib/kora`, `kora.service` |
| service | `KoraService` (LocalSystem; narrowing is a later question, see "Decisions") | `kora.service` (root) |
| channel | named pipe `\\.\pipe\kora-service`, DACL for the allowed SIDs, remote clients rejected | Unix socket `/run/kora/service.sock`, root-owned folder |
| caller identity | the client's token (SID, integrity) on every connection | `SO_PEERCRED` uid on every connection |
| who may use it | `service.json` list, chosen by the installing administrator | same |
| ownership | `owners.json` ledger, keyed per service / firewall / task / database | same file format, `uid:N` owners |
| audit | `audit.log` in the protected folder | `/var/log/kora/audit.log` + journal |

## The allow-list (`privsvc::op::Op`)

`Step` (`admin-step run|undo` for `prepare.db`, `build.db`, `system.firewall`, `system.service`, `system.schedule`),
`DbServer` (start/install Kora's MariaDB), `StartDatabase`, `ScheduleSet/Off` (as the owning user, unelevated),
`InstallGit` (Linux), `UpdateApply` (newer versions only). Not in the list: `install`, `uninstall`, `app uninstall`,
`helper`, `service-host`, update tasks as SYSTEM, downgrades. The install itself runs as the person; only its
administrator steps go to the service. Removing Kora itself is the MSI's / the package manager's job.

The parser is strict, and the service passes the child arguments made from the typed value, never the caller's text
(`Op::child_args`; a test shows the two round-trip).

## Decisions made in this slice

1. **The UAC worker is removed, not kept as a fallback** (slice 6). Reasons in security terms: it is the weakest path
   (an elevated user-writable program); a fallback keeps its whole attack surface alive for the people least able to
   judge it, and the three-mechanism state is what the brief asks to end. Without administrator rights a person
   still uses the per-user install and the panel answers "continue without the administrator parts / show me the
   command"; an administrator who wants the parts runs the MSI (one prompt, a signed program in a protected folder).
2. **Callers are an explicit list**, not "all users" or "all administrators": an administrator is not automatically a
   Kora user, and on a multi-user machine one account's servers must not be another's. Root (Linux) is always served.
3. **No administrators-group folder exception.** The v0.7.0 worker accepted Administrators-owned folders for
   administrator users (made by an elevated Kora). The service only works on folders the caller owns (plus, on Linux,
   the `kora` server account); an administrator uses an elevated terminal for the rest.
4. **Unrecorded items are not removed through the service.** Things made by an older Kora have no ledger entry;
   the migration ("Finish upgrading Kora", slice 4) has an administrator adopt them (`ownership::adopt`).
5. **Windows account: LocalSystem for now.** Narrowing needs a virtual account with exactly the privileges used
   (service control, firewall, ACL changes, scheduled task creation for other users); that list is hard to verify
   without a Windows machine here and is listed for the owner's checklist as a follow-up.

## Slices

1. **Design, protocol, validator, ownership (done):** `privsvc::{op, policy, ownership, paths, audit}`, 26 tests.
2. Linux service + `.deb` + panel routing.
3. Windows service + MSI + panel routing.
4. Migration from 0.6.x / 0.7.0 ("Finish upgrading Kora"; old update tasks rewritten to the unelevated form).
5. Self-update through the service, with rollback.
6. Removal of the dead paths (UAC worker, Linux helper, `winhelper` groundwork).
7. Docs, CI smoke tests on both runners, the Windows checklist.
