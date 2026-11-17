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
  proxy or `tailscale serve` every visitor would look like this computer. The one exception is a proxy you configured
  (`kora panel proxy trust <address>`): forwarded headers (`X-Forwarded-For`, `X-Forwarded-Proto`, the ones you honour) count only from
  that address AND only on a request that carries the shared secret (`X-Kora-Proxy-Secret`, stored as a salted hash, shown once); the
  client address is the right-most `X-Forwarded-For` entry that is not one of your proxies (a client cannot choose its own); every
  proxied request is a remote one (never the launch token, never a local-only operation, never a Tailscale identity), and the
  lockout counts the forwarded address. A wrong secret is refused with the fix; without it nothing changes.
- **HTTPS.** Loopback is plain HTTP (nothing leaves the computer); every other listener serves HTTPS on the same port once there is
  a certificate (rustls with the `ring` provider, TLS 1.2/1.3, HTTP/1.1 only), redirects plain HTTP from other computers (307, to a
  `Host` the panel would accept, never an echoed value), and `--allow-public` refuses to start without HTTPS or a trusted proxy.
  The certificate is one you provide (re-read when the files change; a key file other accounts can read is refused), the one
  `tailscale cert` issues for the tailnet name, or a self-signed one from a local CA kept in the private store (its key never
  leaves it: not in the API, logs, audit log, support data or `export-json`; the CA is never installed or served automatically
  and its fingerprint is shown to compare; ECDSA P-256; 397-day server certificates, renewed before they end). A certificate
  that cannot be loaded never turns HTTPS off: the old one keeps serving and the error is shown. Cookies get `Secure` over TLS;
  `Origin` must use the scheme of the connection (a page served over HTTPS never posts from `http://`); HSTS only for
  certificates you provided or Tailscale's. Not covered: ACME/Let's Encrypt (use a reverse proxy), client certificates, OCSP.
- **Remote sign-in (passkeys, one-time codes).** A remote person proves who they are with a passkey (WebAuthn via `webauthn-rs`,
  pure Rust, no OpenSSL; origin-bound by the browser, fresh challenge each time, state used once and expiring in 5 minutes,
  signature counter checked and saved), or a password (argon2id, strength-checked, dummy verification for unknown names) plus a
  one-time code (RFC 6238; HMAC-SHA1 from RustCrypto; current step and one either side; each step accepted once, enforced in one
  database update) or a recovery code (ten single-use codes, stored as salted hashes). The only bearer link left is the
  enrolment link: 30 minutes, single use (atomically), stored hashed, replaced on reset. Passkeys are bound to a relying-party
  ID, a domain name taken from the `Host` the panel accepted: a bare IP address cannot be one and plain HTTP is refused, and the
  page says to use one-time codes. The same words answer "no such user" and "wrong password"; failures count per source address
  (as for tokens) and per account (5 failures, 15 minutes; a passkey is not blocked by it, so a stranger cannot lock out the owner
  of a passkey). Secrets (password hashes, TOTP secrets, passkey public keys, recovery hashes, the local CA key) live only in the
  private store and are never returned by the API, logged or audited (the audit log records who, how and the outcome, never a
  value). The TOTP secret has to be reversible, so it protects only as much as the store file's permissions. Phishing: a passkey
  answers only for its real origin; a one-time code can be phished, which is why a passkey is the default.
- **API tokens** (for scripts) are not interactive: a role, shown once, stored as a salted hash, optional expiry, accepted only in an
  `Authorization: Bearer` header (never in an address, never a cookie or session, never a way to sign in to the page), still held
  back from destructive operations by the remote-actions switch. Old per-user token links keep working for scripts; they are not
  accepted for signing in once the user has a passkey or one-time codes, nor when an administrator requires them.
- **Sessions and fresh checks.** Remote sessions made with a passkey or code end after 8 hours idle or 3 days; token sessions after
  1 day; the computer's own and Tailscale sessions keep 12 hours / 7 days. Deleting, unregistering, uninstalling, updating Kora, console
  input, schedules and txAdmin from such a session need a passkey or code check from the last 10 minutes; changing how you sign in
  does too. "Signed-in devices" lists a person's sessions (browser, address, times; a public handle, never the cookie) and signs them
  out; resets and removed passkeys end sessions at once. Roles and the remote-actions switch are unchanged.
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
  other, and a file that cannot be read is never replaced). The key values go through one seam and live in the system keychain when there is one
  (Linux Secret Service over the session bus, with the `plain` algorithm; Windows DPAPI, per user): a copy of the Kora folder then holds no
  key in clear, but malware running as you can still ask for it. Locked or unreachable keychains are never prompted and never lose a key (a
  guided "enter it again" instead); see [docs/store.md](docs/store.md). The API shows only a key's name and last four characters.
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
