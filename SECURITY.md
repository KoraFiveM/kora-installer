# Security policy

## Reporting a vulnerability

Please report security problems **privately**, not in a public issue:

1. Go to <https://github.com/KoraFiveM/kora-installer/security/advisories/new> (GitHub private vulnerability reporting).
2. Describe the problem, the affected version (`kora --version`) and how to reproduce it.

We aim to acknowledge a report within 7 days and to tell you what we plan to do within 30 days. Please give us a
reasonable time to release a fix before you disclose publicly.

## Kora Panel security model

`kora panel` runs a small web server on your computer. In brief:

- It binds to `127.0.0.1` (default port 47120), so other computers cannot connect. A public address or every interface
  (`0.0.0.0`) is refused unless you add `--allow-public`; any other address needs remote access turned on first.
- The sign-in link holds a one-time code (works once, within 10 minutes; the long-lived token never needs to sit in a browser's
  history). It is exchanged for an `HttpOnly; SameSite=Strict` session cookie, named per port, so another service on the same
  host cannot read or replace it. Every request must also carry the session's own secret in the `X-Kora-Session` header
  (stream requests that cannot set headers pass it as `?ks=`): a stolen cookie alone is useless. Sessions end after 12 hours
  idle or 7 days in all. Wrong codes and tokens are rate-limited, counted per address (IPv6 per /64) so a stranger only locks
  themselves out. `Authorization: Bearer <token>` is also accepted for scripts.
- Requests that carry proxy headers (`X-Forwarded-For`, `Forwarded`, `X-Real-IP`...) are refused outright: behind a reverse
  proxy or `tailscale serve` every visitor would look like this computer. Use `kora panel --tailscale` instead.
- Every request must carry the panel's own address in `Host` (DNS rebinding). Changing requests must also have a
  matching `Origin` (when sent) and an `X-Kora` header (CSRF).
- Responses carry a Content-Security-Policy that allows only the panel's own files (no external origins, no framing),
  `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`.
- Licence keys, database passwords and webhook addresses are accepted but never returned. Saved licence keys live in
  `secrets.json` in the Kora folder (plain text, readable only by you: `0600` from creation on Linux, inherited permissions
  removed on Windows, atomic writes, never replaced when unreadable); the API shows only a key's name and last four characters.
  A stopped install or the saved answers refer to a saved key by name and never contain the key. Only admins and this
  computer can add, rename, replace or delete keys.
- The panel runs as the installing user, never as administrator: `kora panel` refuses to start elevated (set `KORA_ALLOW_ROOT_PANEL=1` only where root is the sole account). A bare `kora` (also `kora manager` and `kora install` with no options) never elevates: under `sudo` it runs again as the person who typed it, and from an administrator window it only opens a panel that already runs. Administrator work goes to **Kora's administrator service** (below), or, where it is not installed, to a command shown to you.

### Kora's administrator service

One small service per machine does a fixed list of validated administrator operations for the unprivileged per-user panel: Windows service `KoraService` (LocalSystem) and systemd unit `kora.service` (root). It is installed with the program, by the MSI (Windows) or the `.deb` / `kora service install` (Linux), into a folder only administrators can change (`%ProgramFiles%\Kora`, `/usr/lib/kora`), with ONE administrator approval at install time; after that nothing asks again, locally or from another device. It replaced the earlier Linux helper and the Windows UAC worker (which elevated a `kora.exe` in a user-writable folder); neither exists any more. The design and threat model are in `crates/kora-core/src/privsvc/mod.rs`; decisions and the migration are in `docs/privileged-service.md`.

- **Who may use it.** An explicit list of accounts (`kora service list|allow|deny`, administrators only; the installing account is on it). Identity is taken from the connection itself on every connection: the client's token (SID, integrity level) on the Windows named pipe, `SO_PEERCRED` on the Linux Unix socket. Nothing in a request can name an identity. Unreadable settings, a settings file other users can write, a remote client or a low-integrity (sandboxed) program are refused.
- **What may be asked.** A typed allow-list (`privsvc::op::Op`), parsed strictly: one administrator step of an install or delete (`prepare.db`, `build.db`, `system.firewall`, `system.service`, `system.schedule`), start or install Kora's own MariaDB, start the installed database, set or remove an unattended-update task (always as the owning user, unelevated), install Git (Linux), and apply a Kora update to a newer version. Not on the list: a whole install, uninstall, removing Kora, update tasks as the system account, downgrades, any other program. The service never forwards the caller's text: it rebuilds the arguments from the typed value (a test checks every one is understood by the real command line).
- **Never trusts what the caller controls.** The folder is re-checked on the privileged side without following links, must belong to the caller and hold a Kora install record; ports come from the record read there (never below 1024, never remote-administration or database ports); the name used to undo comes from the person's server list. Steps never write into the folder; the Linux hand-over of a server folder to its own account uses open folder handles, never `chown -R <path>`.
- **Ownership.** `owners.json` (administrators only) records which account every service, firewall rule set, update task and database was made for. Making something needs "not owned by someone else", removing needs "owned by the caller"; things Kora made before the records existed are only changed after an administrator finishes the upgrade (`kora service migrate`, run by the package and the MSI), which records them only for accounts that can be shown to own them.
- **Clean child.** Operations run the protected program with an environment built from nothing (system-only `PATH`, no `KORA_*` override, no `GIT_*`, the caller's own profile folders looked up by the service), one at a time, with a time limit per operation, cancelled when the caller goes away. The service refuses to start unless its program is in the protected folder and not writable by others. Windows pipe: owner SYSTEM (the panel checks it before sending anything), first instance (a squatter makes the service refuse to start), remote clients rejected, network logons denied, and the listed accounts get only read data, write data and read attributes, not "create pipe instance", so an allowed account cannot make a look-alike pipe.
- **Audit.** One line per operation, allowed or refused (when, which account, what, the result), in `%ProgramFiles%\Kora\data\audit.log` and `/var/log/kora/audit.log` (also the journal). Never a path, an argument value or a secret.
- **Updates.** The service applies a Kora update itself (same release feed and checksum check as `kora self-update`), keeps the running program, checks that the new one starts (else puts the old one back at once), restarts, and rolls back after three starts that do not stay up. Provenance (`gh attestation verify`) is checked by the release job, not by the service.
- **Secrets** (licence key, database password) travel only in a request's environment, never on a command line, are masked in output and never logged; Kora's own MariaDB administrator login stays on the privileged side.
- **What it does not protect against.** An allowed account (and any program running as it) can do the allow-listed things without a prompt: open firewall ports of 1024 and above for its own servers, register its own servers' services, create its own databases, start Kora's database, apply an official Kora update. Each is limited to that account's own items. Remove the account (`kora service deny`) or uninstall the service to take that away. An administrator who answers a prompt for someone else's account is not part of this design any more: there is no prompt except the one for the installer itself.

Full user-facing description: [docs/panel.md](docs/panel.md). Report a panel vulnerability privately, in the same way
as any other (see above). Include the version, your platform, and whether the panel was local or remote.

## Supported versions

Kora Installer is pre-release software. Only the newest release receives security fixes.

## In scope

- **Privilege elevation:** UAC / `sudo` handling, anything that runs with more rights than needed, services and scheduled tasks.
- **Secrets handling:** the txAdmin password, database passwords, licence key and tokens (files, permissions, logs, clipboard), including the saved-keys file `secrets.json`.
- **Download verification:** SHA-256 checks and HTTPS enforcement in `kora` and in `install.sh` / `install.ps1`, and the release artifacts.
- **The panel:** authentication, session handling, the `Host` / `Origin` / `X-Kora` checks, the Content-Security-Policy, anything that lets a web page or another local user act on your servers, and anything that returns a secret to the browser.
- Overwriting or deleting files the installer should not touch.

## Out of scope

- Problems in third-party software that Kora installs (FXServer, txAdmin, MariaDB, git); report those upstream.
- The unsigned Windows executable triggering SmartScreen (known; see the README and CODE_SIGNING.md).
- Issues that need an attacker who already has administrator/root rights on the machine.
- Exposing the panel to an untrusted network with `--bind` over plain HTTP against the documented advice.
