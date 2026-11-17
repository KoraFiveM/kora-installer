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
- **txAdmin through the panel (the gateway).** txAdmin is full control of the game server (resources and `exec` run code),
  so opening it needs the **admin** role and counts with the risky actions: a remote sign-in made with a user token needs
  `kora panel remote-actions on`; a Tailscale administrator does not. The panel opens one extra TCP port per server for it
  (txAdmin's port + 10000, only on the addresses the panel itself listens on). Nobody gets in without a one-time,
  one-minute ticket made by an authenticated panel request (session cookie and per-session secret, the role check and the
  remote gate); the ticket becomes a gateway cookie that names that panel session and is checked again on every request and
  every 20 seconds on an open live console, so signing out, removing the user, a role change or turning the switch off ends
  it. Requests go only to `127.0.0.1` and the txAdmin port the server list holds for that server (never chosen from the
  request: absolute-form targets and other `Host` values are refused). The panel's cookies and secret are never sent to
  txAdmin, txAdmin's cookies are renamed per gateway (two servers never share a session; nothing txAdmin sets can replace a
  panel cookie), and `x-txadmin-token` / `x-txadmin-identifiers` (which txAdmin treats as FXServer's own, no login needed),
  `Authorization`, proxy and hop-by-hop headers are dropped. Proxy headers refuse the request, and WebSocket handshakes
  need an `Origin` equal to the gateway's own `Host`. What stays true: txAdmin sees every visitor as 127.0.0.1, so its
  own sign-in rate limit is shared and its flood protection does not apply; txAdmin has no single sign-on, so a person
  who may open it still signs in to txAdmin itself. Checked against txAdmin 8.1.2; the checklist for a real Windows
  run is in `docs/windows-test-checklist.md`.
- **Reachability check and port forwarding.** Nothing outside the computer is contacted unless the person allowed that
  lookup (Cfx.re's public server list with the server's own join code; `api.ipify.org` for the public address). Router
  discovery (UPnP, PCP, NAT-PMP) stays on the local network: the router's description must come from the address that
  answered, control addresses on any other host are refused, replies are size-limited and parsed strictly. Forwarding is
  off by default, per server, admin only; it only ever maps that server's own game port (TCP and UDP, 1024 or higher)
  to this computer, with a lease that is renewed while the server runs and removed when it stops or is deleted.
- Responses carry a Content-Security-Policy that allows only the panel's own files (no external origins, no framing),
  `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`.
- Licence keys, database passwords and webhook addresses are accepted but never returned. Saved licence keys live in
  Kora's data file (`store/kora.db` in the Kora folder, see [docs/store.md](docs/store.md); plain text, not encrypted, as private as `secrets.json`
  was: the `store` folder is `0700` and its files `0600` from creation on Linux, including SQLite's `-wal`/`-shm` files, and on
  Windows its inherited permissions are removed so everything created in it inherits that; links are refused, files made by `sudo`
  inside someone else's folder go back to its owner; every change is one transaction, so two programs cannot overwrite each
  other, and a file that cannot be read is never replaced). The key values have their own table so they can move to the system
  keychain later. The API shows only a key's name and last four characters.
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
- Remote access (`--bind <ip> --remote`) is off by default and uses plain HTTP: use Tailscale, WireGuard or an SSH tunnel, never
  the open internet. Roles are viewer / operator / admin. Remote administrators can install and manage servers.
  Tailscale administrators can also delete, uninstall and send console input; for token sessions those stay off until `kora panel remote-actions on`
  is run on the machine, and deleting always needs the server name typed. Reasoning: a Tailscale identity is checked by the local Tailscale
  service (a person on a device they signed in), never from request headers; a token is a bearer secret that can leak, and a leaked token that
  can only install can be undone by removing the new server, while one that can delete cannot be undone.
- The old machine-wide "Kora app" (administrator shortcuts, a copy in `Program Files` or `/opt/kora`) is no longer installed. The
  per-user shortcuts that replace it never need administrator rights and are not "Run as administrator"; "Tidy up old shortcuts"
  removes the old copy, which needs administrator rights (a command shown to you); "Finish upgrading Kora" also sets aside your own earlier copy of the program.
- Hub nodes authenticate with bearer tokens; treat them like passwords.
- Kora's own Git (MinGit, Windows) and GitHub CLI are downloaded only from `https://github.com/` release assets at a version
  and SHA-256 pinned in the source (`crates/kora-core/src/tools/pins.rs`, updated with `scripts/update-tool-pins.sh`).
  The file is verified before anything is unpacked, unpacked without links or paths that leave the folder, and moved into
  place in one step. They live in the user's own Kora folder and need no administrator rights. A Git or GitHub CLI already on
  the computer is preferred: Kora looks for it on the `PATH` (absolute entries only, never the current folder) and then in the usual
  install folders on Windows, and starts the copy it found by its full path. On Windows the folder Kora itself runs from is never
  searched. On Linux it is searched if you put it on your `PATH` (`~/.local/bin` often is), like any other program there.
- Switching a resource to another branch or a pull request (`kora branch`, `kora pr`, a resource's page) runs `git` and `gh` as the installing user, only inside the server's `resources` folder (a folder that points elsewhere is refused), and only with checked values: branch names and pull request numbers are validated before they reach a command line, and an address is only accepted when it is the resource's own repository or your own copy. It never forces anything, never deletes a branch or a stash, and never touches changes you have not saved without your explicit choice. A pull request is other people's code that then runs on your server (the page says so every time): trying one needs the admin role, like other changes to the server, and that includes a remote administrator signed in with a token, who can therefore make the server run code from any open pull request of that resource's repository or your own copy. **On a server that follows released versions this is allowed too** (it used to be refused), because testing a pull request on the real server is useful; it is a deliberate choice: the panel says "this resource will run unreleased code until you put it back", asks for a tick, the update check and the scheduled update leave the resource alone (never merging into it) and say why, every list shows the resource with the "not on its normal version" badge, and **Back to the released version** returns to exactly the pinned commit. Kora itself runs nothing from a resource's files: `git` only uses the hooks and settings in the folder's own `.git`, which a checkout does not change, and credentials are only offered to github.com.
- **Whose GitHub account acts.** Everything done on GitHub from the panel or the command line (push, comment, review, merge, issue comment, opening a pull request, forking) is done by the one GitHub account `gh` on the computer that runs Kora is signed in as. That identity is **shared by every panel user**: the panel says "This is done on GitHub as <login>" beside every such button and in every answer. All of these need the admin role; push and merge are also in the "destructive" class, so a remote administrator signed in with a token is refused them unless the owner turned remote actions on (`kora panel remote-actions on`) or signed in through Tailscale. Reading pull requests, issues, releases and diffs needs the operator role (a diff can contain a resource's configuration, so it is not shown to viewers). The GitHub work never force-pushes, never deletes a remote branch, never rewrites history and never passes `--admin` to a merge; `git push` is only `git push --set-upstream origin <the checked-out branch>` and git itself refuses anything that is not a fast-forward. Commit messages, comments and review texts are sent as argument values (never through a shell), are length-limited, and GitHub text shown in the panel is plain text. A pull request to merge is read again from GitHub immediately before the merge, and it is refused when the account has no write permission, the pull request is not open, or the repository does not allow the chosen method.
- **Making your own copy (fork)** creates repositories in the account or organisation you choose (an organisation is checked for the right to create repositories first), re-points the resource's remotes and keeps its commit, branch and changes. It uses only `gh repo fork` and `git remote`; nothing is deleted, and the original stays as `upstream`.
- Remote operators can list the saved licence keys (name and last four characters, and which servers use them) and point a server at one of them by name; they cannot see, paste, add or change a key. Remote administrators (token or Tailscale) can also choose the folder of a new server, anywhere the panel's user can write.
- A stopped install is kept in memory for six hours, with its licence key and database password. When it used a saved key it is also written to Kora's data file for three days without any secret, so it can carry on after a restart (an install that was running when the panel died is written the same way and offered after the restart); a pasted key that was not saved never leaves memory. Job records and their events are kept in the data file for 30 days and never hold a secret (key-shaped text and fields called key, password, token or secret are blanked when written).
- Every change made through the panel or a `kora` command is recorded in an append-only audit log in the data file (when, operation, server, who with provider and role, result; never a secret or request contents; shown to administrators in Settings and by `kora audit`; kept 365 days). It is not tamper-proof against someone who can open the private data file.
- `kora panel` and a bare `kora` print a sign-in link with a one-time code. If the running panel cannot give one (an older version, or it does not answer), the link carries the panel's own token instead: do not paste that link into chats or tickets.

Full user-facing description: [docs/panel.md](docs/panel.md). Report a panel vulnerability privately, in the same way
as any other (see above). Include the version, your platform, and whether the panel was local or remote.

### The desktop app (tray icon and window)

The optional `kora-desktop` program is a client of the panel, not a new server: it opens nothing to the network and the panel's
rules are unchanged. Its window shows only the panel's own origin (`http://127.0.0.1:<port>`); every other web address opens in
the default browser and every other kind of address is refused, so no remote page ever runs inside the window or sees the
app's small bridge. The bridge is an allow-list (choose a file or folder, open a web link in the browser, show a notification
that can only open a panel page, and one fixed administrator action by name) and cannot run a program or read or write a
file. The panel's token is handed from `kora` to the app over a pipe, kept in memory and only sent to the panel over loopback in
a header; it is never logged or written down. The app never runs with administrator rights and the page cannot choose
what is elevated. The full threat model is in [docs/desktop-app.md](docs/desktop-app.md). Its GUI toolkit is built only into
that separate program, from a separate locked dependency list that has its own supply-chain check.

## Supported versions

Kora Installer is pre-release software. Only the newest release receives security fixes.

## In scope

- **Privilege elevation:** UAC / `sudo` handling, anything that runs with more rights than needed, services and scheduled tasks.
- **Secrets handling:** the txAdmin password, database passwords, licence key and tokens (files, permissions, logs, clipboard), including the saved keys in the data file `store/kora.db` and its permissions.
- **Download verification:** SHA-256 checks and HTTPS enforcement in `kora` and in `install.sh` / `install.ps1`, and the release artifacts.
- **The panel:** authentication, session handling, the `Host` / `Origin` / `X-Kora` checks, the Content-Security-Policy, anything that lets a web page or another local user act on your servers, and anything that returns a secret to the browser.
- Overwriting or deleting files the installer should not touch.

## Out of scope

- Problems in third-party software that Kora installs (FXServer, txAdmin, MariaDB, git); report those upstream.
- The unsigned Windows executable triggering SmartScreen (known; see the README and CODE_SIGNING.md).
- Issues that need an attacker who already has administrator/root rights on the machine.
- Exposing the panel to an untrusted network with `--bind` over plain HTTP against the documented advice.
