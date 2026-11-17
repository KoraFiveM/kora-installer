# The Kora privileged service (design, decisions, migration)

Status: **built end to end** on branch `nx/privileged-service`. The Linux half is covered by unit tests, real-socket
tests and the `admin-service` Linux smoke job; the Windows half is type-checked and unit-tested on Linux and exercised
for real only by the `msi` Windows smoke job and the owner's checklist (`docs/windows-test-checklist.md`, "Kora's
administrator service and the MSI"). The threat model is in the module documentation of `crates/kora-core/src/privsvc/mod.rs`
and, in user terms, in SECURITY.md.

## What and why

Kora used to get administrator rights three ways: an optional Linux helper, a Windows worker started through a UAC prompt
that elevated a `kora.exe` in a user-writable folder, and elevated terminals. The v0.7.0 security review found elevating a
user-writable program, driven over a pipe by an unprivileged process, the weakest point, and a remote install on Windows
with nobody at the desktop could not work at all.

Now: **one small service** per machine, installed by a proper installer into a protected folder, doing a fixed allow-list
of validated operations for the unprivileged per-user panel.

| | Windows | Linux |
|---|---|---|
| installer | MSI (WiX v5, built in CI): Program Files, service, PATH, shortcuts, Apps & features | `.deb` (dpkg-deb, built in CI) or `kora service install` (verified release program, same layout) |
| service | `KoraService`, LocalSystem, auto start, restart on failure | `kora.service`, root, `Restart=always` |
| channel | named pipe `\\.\pipe\kora-service` | Unix socket `/run/kora/service.sock` |
| caller identity | client token (SID, integrity) after the first read | `SO_PEERCRED` uid |
| who may use it | `%ProgramFiles%\Kora\data\service.json` | `/etc/kora/service.json` |
| ownership ledger | `%ProgramFiles%\Kora\owners.json` | `/var/lib/kora/owners.json` |
| audit log | `%ProgramFiles%\Kora\data\audit.log` | `/var/log/kora/audit.log` + journal |
| child | `%ProgramFiles%\Kora\kora.exe` | `/usr/lib/kora/kora` (or `/opt/kora`) |

Code map (`crates/kora-core/src/privsvc/`): `op` (typed allow-list and strict parser), `policy` (accounts, `authorise`,
pipe descriptor), `ownership` (folder rule, ledger rules, ledger), `dispatch` (the per-request pipeline over a `Host`
trait), `conn` (serving a connection and the client conversation, transport independent), `unixsvc` / `winsvc` (the OS
layers), `exec` (clean-environment child runner), `update` (update with rollback), `migrate` / `migrate_os` (finish
upgrading), `install` / `wininstall` / `manage` (install, enrol, allow/deny), `problems` (what the person reads),
`paths`, `audit`, `wire`, `identity`, `winapi`. The panel's one seam is `crates/kora-web/src/elevated.rs`.

## Decisions

1. **The UAC worker is removed, not kept as a fallback.** It is the weakest path (an elevated user-writable program), a
   fallback keeps its whole attack surface alive for the people least able to judge it, and the brief asks to end the
   three-mechanism state. Without administrator rights a person uses the per-user install and the panel offers
   "Install Kora's administrator service" (Windows: launches the checked MSI under one prompt; Linux: shows
   `sudo kora service install`), "continue without the administrator parts", or "show me the command".
2. **Callers are an explicit list**, chosen at install time (the installing account by default), changed with
   `kora service allow|deny|list` (administrators only). An administrator is not automatically a Kora user; on a shared
   machine one account's servers must not be another's. Root is always served.
3. **No administrators-group folder exception.** The service only works on folders the caller owns (plus, on Linux, a
   folder owned by one of Kora's dedicated server accounts when the ledger shows that server's service as the caller's).
4. **Unrecorded items are not changed through the service.** Things made before the ledger existed are adopted by an
   administrator's `kora service migrate`, only for accounts that can be shown to own them (folder inside their home or
   owned by them, the thing exists, nobody else claims it).
5. **Linux uses the same model as Windows** (open question from slice 1): the install plan runs as the person; only the
   administrator steps (`prepare.db`, `build.db`, `system.firewall`, `system.service`, `system.schedule`, and git
   install) go to the service. The dedicated server account stays. The risky part, the recursive `chown` of a folder the
   person controls, is done through open folder handles (`fsutil::chown_tree_no_follow`), not by path, and deleting a
   server hands the folder back to the caller so the panel deletes the files as that person. Reason: it is the
   simplest model that closes the link-swap race and keeps one code path on both systems.
6. **Settings' "remove everything Kora set up" and the old-app tidy stay administrator-terminal commands**, not service
   operations: rare, destructive, and not needed remotely; the per-server removal (delete) does go through the service.
7. **Service data lives in `%ProgramFiles%\Kora\data` (not `%ProgramData%`)**: only administrators can create anything
   in Program Files, so the folder cannot be squatted by a user before the installer runs.
8. **The Windows pipe grants the listed accounts exactly read data, write data and read attributes (`0x120083`), not
   `FILE_APPEND_DATA`** (= create a pipe instance, part of `GENERIC_WRITE`), so an allowed account cannot make a look-alike
   pipe; the panel also checks the pipe's owner is SYSTEM before sending anything.
9. **Updates through the service** reuse `kora self-update --yes` as the child (same release feed and checksum), with a kept
   copy of the running program, an immediate "does the new one start" check, a pending-update record, a restart by the
   service manager (`Restart=always`; Windows: a failure exit code plus restart actions on non-crash failures) and a
   rollback after three starts that do not stay up.
10. **MSI shortcuts are all-users** (Start Menu, and an optional public Desktop one), not per-user: a per-machine package
    installs for every account; per-user shortcuts are made by `kora app install` when someone runs Kora.

## The service and the store (`docs/store.md`)

- The privileged side never opens a person's `store/kora.db`: root would create `-wal` / `-shm` there and lock the person out. A child of
  the service calls `store::forbid_in_this_process()` at start (every way of opening a store then fails and is counted);
  `crates/kora-core/tests/privileged_never_opens_a_store.rs` runs every administrator step and the migration in service mode and fails if
  anything tried, or if a file in the store folder changed.
- What a child needs to know about a server comes from the server FOLDER the service already validated (its install record) or from the
  request: update schedules take the folder (`kora schedule set|off <folder>`), undo steps carry the name and ports. The shared
  root-owned program copy is kept when a server is deleted through the service (it cannot ask the person's list whether another server uses it).
- The migration (`kora service migrate`) reads the person's server list with `store::peek::registered_server_folders`: read-only and
  immutable, no file created or changed (tested), and the old `servers.json` is accepted on a machine that has not run the new Kora yet.
  An immutable open ignores a write-ahead log, so a row written moments ago may be missed; the answer only names folders that are then checked
  from the system.
- Audit: the service's log is separate from the store's Activity log; see `docs/store.md`, "The audit log".

## Migration (0.6.x / 0.7.0 to this layout)

`kora service migrate` (run by the `.deb`'s `postinst` and the MSI, safe to repeat; `--dry-run`, `--quiet`):

- Linux: stops and removes `kora-helper.service`, `/usr/local/libexec/kora/kora`, `/etc/kora/helper.json`; the account in
  `helper.json` is allowed in the new service. Servers installed the old way (root-run, folder owned by the server's own
  `kora-<name>` account, inside the person's home) are recorded in the ledger for that person, so they stay manageable.
- Windows: update tasks `\Kora\Update <name>` that run as a user with `HighestAvailable` are rewritten to
  `LeastPrivilege` (system-account tasks are left and noted); the 0.7.0 worker's ledger entries already carry SIDs and are
  kept; pre-ledger items are adopted the same way as on Linux.
- Per person (`kora service finish-upgrade`, also done by "Tidy up old shortcuts"): the person's own earlier program
  (`%LOCALAPPDATA%\Kora\bin\kora.exe`, `~/.local/bin/kora`) is set aside as `.previous`, removed from their PATH, and the
  background panel's start-up file points at the installed program.
- Never touched: server folders, databases, services, firewall rules, saved keys, the registry of servers.
- Way back: install the earlier release's installer; the set-aside copy is still there as `.previous`; the old helper's
  files are the only thing removed outright (the 0.7.0 `.deb`-less layout can be re-created with that release's
  `kora helper install`).

## Packaging and CI (what was added)

- `packaging/linux/build-deb.sh` + `deb/{postinst,prerm,postrm}`: built with `dpkg-deb` (already on the runner, nothing
  fetched). `release.yml` job `package-linux` builds `kora-linux-x86_64.deb` (+ `.sha256`); `verify` installs it for real
  with `scripts/smoke-linux/07-service.sh`.
- `packaging/windows/kora.wxs` + `build-msi.ps1`: WiX v5 fetched in CI as a pinned .NET tool
  (`dotnet tool install --global wix --version 5.0.2`, plus the Util extension), not vendored.
  `release.yml` job `package-windows` builds `kora-windows-x86_64.msi` (+ `.sha256`); the attestation covers both.
- Smoke: `linux-smoke.yml` job `admin-service` (`07-service.sh`: install the `.deb`, non-root accounts, refusals, a v0.7.0
  layout upgrade, remove and purge); `windows-smoke.yml` job `msi` (`14-msi.ps1`: standard accounts through the Task
  Scheduler, refusals, pipe rights, upgrade, uninstall). The Task Scheduler account helper in `14-msi.ps1` should be
  consolidated with `scripts/smoke/lib-accounts.ps1` from branch `nx/release-trust`.
- `install.sh` / `install.ps1` offer the system install first; `KORA_SYSTEM=1|0`, `-System` / `-PerUser`.

## Operations added after the first release

- **Scheduled backups as an operating-system job** (`backup-job set|off <folder>`). `kora-backup-<slug>` (systemd timer + `User=<caller>` unit / Task Scheduler task, `InteractiveToken`, `LeastPrivilege`) runs `<kora> backup create <folder> --scheduled` as the calling account: the account comes from the connection, the program is the root-owned pinned copy, a time is always given (daily or weekly, validated like the update schedule), never a passphrase. Own ledger key `backup:<slug>`; removed by deleting the server (through the service, before its settings are forgotten) and by `kora uninstall`. The privileged child never opens the store; the job it makes does, as the person. The panel's own scheduler stands down for a server whose `BackupStatus.os_job` is set; the job does nothing when the settings say off, and marks its slot so the panel's scheduler cannot repeat it.
- **Handing restored files to the server's own account** (`hand-over <folder>`). Linux: `fsutil::chown_tree_no_follow` to `kora-<name>`; Windows: the `NT SERVICE\kora-<name>` rights again. The account is never taken from the request or from the folder's records as written: it is the one Kora makes for the name in the folder's install record, must exist and be a system account. Called at the end of a restore (before the server starts again) and of an import; the result says plainly when it could not be done, with the command.
- **Reading the service's own audit** (`audit-read`, read-only, answered by the service itself, no child, not written to the log it reads). Only lines about the calling account unless the caller is root, an elevated Windows token or in `sudo`/`wheel`/`admin`; at most 500 lines of at most 300 characters from the last 512 KB, each passed through `support::redact`. The panel shows them in Settings, Activity log, labelled "Done with administrator rights" and merged by time (`GET /audit/service`, administrators only; `kora audit --service`).
- **Windows: a standard account approved with an administrator's password.** The installer passes Windows Installer's `ClientProcessId` to `kora service allow-installer`; the SYSTEM side reads that process's token and, when it is elevated, its parent's (the program that asked for the permission), and allows that person only when `privsvc::installer_user::decide` finds an interactive session, an ordinary account and matching sessions. Anything unclear allows nobody extra (today's behaviour). Unverified on a real Windows: see the checklist (S3b).

## Not done / follow-ups

- **Windows service account.** LocalSystem. A virtual account with only the privileges used (service control, firewall,
  ACL changes, task creation for other users) is worth trying, but cannot be verified without a Windows machine.
- **Provenance attestation check in the service.** The child verifies the release checksum like `self-update`; the
  `gh attestation verify` check stays in the release job.
- **Signing.** The MSI and `kora.exe` are unsigned until the release-trust workstream's signing lands (the permission
  prompt for the installer shows an unknown publisher).
- **Everything Windows is unverified by a real run here.** Expect the first CI runs to need small fixes (WiX attribute
  details, the Task Scheduler account helper, `Start-Process -Verb RunAs` in `install.ps1`, service failure-action
  flags); the smoke scripts print the MSI log, `sc qc/query`, the System event log and the service's own files on failure.
- **DB steps on Linux through the service** (a non-elevated install uses the service for Kora's database admin login, the
  root socket) follow the Windows-first `database_elevated` path; the Linux smoke does not install MariaDB, so that path
  is covered by unit tests only.
