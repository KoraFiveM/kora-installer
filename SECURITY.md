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
- The panel runs as the installing user, never as administrator: `kora panel` refuses to start elevated (set `KORA_ALLOW_ROOT_PANEL=1` only where root is the sole account). A bare `kora` (also `kora manager` and `kora install` with no options) never elevates: under `sudo` it runs again as the person who typed it, and from an administrator window it only opens a panel that already runs. Privileged actions are a command shown to you, a
  helper (`kora helper`, Linux) that accepts only Kora's fixed list of privileged actions and checks the calling user, or on
  Windows a short-lived administrator worker started through the normal Windows permission prompt (see below).
- **Windows administrator worker.** For an install (one prompt for all its administrator steps) or one administrator action,
  the panel starts its own `kora.exe` elevated with the Windows prompt (`runas`) and talks to it over a named pipe:
  - The pipe has a random name, is created as the first and only instance, refuses network clients, and its security descriptor
    lets only elevated administrators and SYSTEM open it (the panel's own account cannot loosen it). Another local account, or a
    program running as you without elevation, cannot connect.
  - Both ends check each other before anything is sent: the worker serves only the panel process whose id it was given AND that
    runs the same program file; the panel serves only the process Windows started for it. The worker opens the pipe for
    identification only, so the panel can never act with the worker's rights.
  - The worker runs only Kora's fixed list (the Linux helper's checks, then the `kora` command line's own definition): the
    update schedule, Kora's self-update to a newer version, removing the old app or Kora's system changes, and one administrator
    step of an install or delete (`kora admin-step`), never a whole install, never any other program, never through a shell.
    Install steps only work on a server folder that holds a Kora install record and belongs to you (or, for an administrator, a
    folder an elevated Kora made). The install's plan, state and files stay yours; the elevated side writes nothing into the
    server folder and hands back what to record.
  - Secrets never go on a command line: the licence key is not needed by the worker; Kora's own MariaDB administrator login stays
    on the elevated side, and only the new server's database login comes back (into `database.json`, as before).
  - The worker exits when its batch is done, when the panel goes away (the pipe breaks), or after 90 minutes without work.
  - When a remote session (another device, Tailscale, a token) needs administrator rights, the panel does not make Windows ask on a
    desktop nobody may be watching unless the person asks for it.
  - What it does not protect against: malware already running as you can show its own permission prompt or tamper with the panel,
    as it could with any program you run; Windows does not treat the permission prompt as a boundary against your own account.
    When a standard account's prompt is answered with an administrator's password, that administrator lets this account's server
    run as a Windows service and its firewall rule be opened, exactly as typing the shown command into an administrator terminal
    would; files and settings stay the standard account's.
- Remote access (`--bind <ip> --remote`) is off by default and uses plain HTTP: use Tailscale, WireGuard or an SSH tunnel, never
  the open internet. Roles are viewer / operator / admin. Remote administrators can install and manage servers.
  Tailscale administrators can also delete, uninstall and send console input; for token sessions those stay off until `kora panel remote-actions on`
  is run on the machine, and deleting always needs the server name typed. Reasoning: a Tailscale identity is checked by the local Tailscale
  service (a person on a device they signed in), never from request headers; a token is a bearer secret that can leak, and a leaked token that
  can only install can be undone by removing the new server, while one that can delete cannot be undone.
- The old machine-wide "Kora app" (administrator shortcuts, a copy in `Program Files` or `/opt/kora`) is no longer installed. The
  per-user shortcuts that replace it never need administrator rights and are not "Run as administrator"; "Tidy up old shortcuts"
  removes the old copy, which needs administrator rights through the helper or a command shown to you.
- Hub nodes authenticate with bearer tokens; treat them like passwords.
- Kora's own Git (MinGit, Windows) and GitHub CLI are downloaded only from `https://github.com/` release assets at a version
  and SHA-256 pinned in the source (`crates/kora-core/src/tools/pins.rs`, updated with `scripts/update-tool-pins.sh`).
  The file is verified before anything is unpacked, unpacked without links or paths that leave the folder, and moved into
  place in one step. They live in the user's own Kora folder and need no administrator rights. A Git or GitHub CLI already on
  the computer is preferred: Kora looks for it on the `PATH` (absolute entries only, never the current folder) and then in the usual
  install folders on Windows, and starts the copy it found by its full path. On Windows the folder Kora itself runs from is never
  searched. On Linux it is searched if you put it on your `PATH` (`~/.local/bin` often is), like any other program there.
- Switching a Development resource to another branch or a pull request (`kora branch`, `kora pr`, the GitHub tab) runs `git` and `gh` as the installing user, only inside the server's `resources` folder (a folder that points elsewhere is refused), and only with checked values: branch names and pull request numbers are validated before they reach a command line, and an address is only accepted when it is the resource's own repository or your fork. It never pushes, never forces, never deletes a branch or a stash, and never touches changes you have not saved without your explicit choice. A pull request is other people's code that then runs on your server (the pull request dialog says so): trying one needs the admin role, like other changes to the server (adding resources, rolling back), and that includes a remote administrator signed in with a token, who can therefore make the server run code from any open pull request of that resource's repository or your fork. Kora itself runs nothing from a resource's files: `git` only uses the hooks and settings in the folder's own `.git`, which a checkout does not change, and credentials are only offered to github.com. Production servers cannot be switched.
- Remote operators can list the saved licence keys (name and last four characters, and which servers use them) and point a server at one of them by name; they cannot see, paste, add or change a key. Remote administrators (token or Tailscale) can also choose the folder of a new server, anywhere the panel's user can write.
- A stopped install is kept in memory for six hours, with its licence key and database password. When it used a saved key it is also written to Kora's data file for three days without any secret, so it can carry on after a restart (an install that was running when the panel died is written the same way and offered after the restart); a pasted key that was not saved never leaves memory. Job records and their events are kept in the data file for 30 days and never hold a secret (key-shaped text and fields called key, password, token or secret are blanked when written).
- Every change made through the panel or a `kora` command is recorded in an append-only audit log in the data file (when, operation, server, who with provider and role, result; never a secret or request contents; shown to administrators in Settings and by `kora audit`; kept 365 days). It is not tamper-proof against someone who can open the private data file.
- `kora panel` and a bare `kora` print a sign-in link with a one-time code. If the running panel cannot give one (an older version, or it does not answer), the link carries the panel's own token instead: do not paste that link into chats or tickets.

- **Server backups hold secrets, by necessity** (the licence key, the database login, the txAdmin admin and password hashes): the restore could not work without them. A backup is created owner-only (0600; on Windows the default folder gets the protection Kora gives its data file and a chosen folder gets `icacls` on the finished file) inside a private folder, and is never inside the server's own folder. Only an **administrator** can download, restore, delete or move a backup (a remote sign-in made with a token also needs `kora panel remote-actions on`); an operator can make one, which only writes into the configured folder, and cannot choose another folder (it would write anywhere the panel's user can). **Password protection** uses the `age` format through the `age` crate (passphrase mode: scrypt and ChaCha20-Poly1305 in 64 KiB authenticated chunks); there is no home-made cryptography. A lost password cannot be recovered, Kora never stores it, and it is never put in a command line, a log, a job event, the activity log or an answer: it travels once in a request body, and the CLI reads it from standard input or `KORA_BACKUP_PASSWORD`. Scheduled backups cannot be password protected (nobody can type it), so the folder they go to must be one only the owner can read. A file you did not make is treated as hostile: the archive is verified against its checksums before anything is placed, names inside it must be plain relative paths (no `..`, links, devices), the amount unpacked is capped, repository addresses and branch names inside it are checked before they reach `git` (no option injection, no `file://`), the database name read from `database.json` is checked before it reaches a command line, and nothing in it is ever executed. The database is dumped and restored with the server's own login (never the administrator's), the password in the environment of the child process only. An imported file is uploaded to a private folder in the Kora home (owner-only, streamed to disk, at most 20 GB, removed after the import and after a day), and a path on the computer can only be named from that computer's own session. Restoring replaces a running server's data, so it takes a safety copy first and keeps today's `secrets.cfg` and `database.json`. The import never acts on another machine: removing the server from the old computer is a list of instructions.
- **The support bundle** is made by one redaction function (`crates/kora-core/src/support/redact.rs`, threat model in its module docs): files known to carry secrets are never read, everything else is scrubbed by secret-shaped patterns and by setting names, and a result that still looks secret is dropped. It is shown to the person before it is saved, saved owner-only on the computer, administrators only, and never uploaded. What it does **not** hide: server names, ports, folder names, public IP addresses, resource names and ordinary log text.

Full user-facing description: [docs/panel.md](docs/panel.md). Report a panel vulnerability privately, in the same way
as any other (see above). Include the version, your platform, and whether the panel was local or remote.

### Windows administrator worker

On Windows the panel (never elevated) gets administrator work done through a short-lived elevated copy of Kora that it
starts with the normal Windows permission prompt (design and threat model: `crates/kora-core/src/helper/worker.rs`).
What it guarantees, and what it does not:

- **Who it obeys.** It talks only over a private named pipe that only elevated administrators and SYSTEM may open
  (random name, first instance, remote clients refused), only to the panel process that started it, and only when that
  process runs the very same program file (compared by volume and file identity). It runs only Kora's fixed list of
  administrator steps, each checked against Kora's own command-line definition.
- **What it runs.** From the prompt until the session ends, nobody can replace or rename Kora's program file, so the
  prompt and every elevated step start the file the panel runs. Each step runs as a child with an environment built
  from nothing: system folders from Windows itself, a `PATH` of system folders only, the panel user's own profile, and
  only the licence key and database password variables the step needs. Override and test variables are ignored when
  elevated. It never takes a database client from `PATH`. One session and one elevated step at a time; a step is
  stopped when the panel goes away or after 45 minutes.
- **What it works on.** A server folder must hold a Kora install record and belong to the person the panel runs as
  (re-checked in the elevated step itself, no links on the way); servers are named from the person's own list, never by
  folder path. A record in `%ProgramFiles%\Kora\owners.json` (administrators only) says which Windows account each
  service, firewall rule set, update task and database was made for, and another account's are never changed. The
  firewall step opens no port below 1024 and no remote-desktop, remote-management or database port. The elevated
  uninstall never deletes folders (the panel deletes them as the person). Server services keep only an ordinary user's
  privileges (no "impersonate a client"), and the update task runs without elevation.
- **What it does not protect against.** UAC is not a boundary between a person and their own programs. A harmful program
  already running as the person can replace `kora.exe` in the person's folder before the panel starts, or inject into the
  running panel and use a session the person approved until it ends; approving the prompt is like typing the shown
  command into an administrator terminal. On an upgraded 0.6.x machine the old update task runs with full rights until
  the schedule is changed once (an elevated Kora hardens `git` meanwhile). The pipe has no read time-out: a worker that
  hangs without exiting holds the panel's administrator session until the panel restarts.

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
