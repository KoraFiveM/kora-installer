# Security policy

## Reporting a vulnerability

Please report security problems **privately**, not in a public issue:

1. Go to <https://github.com/KoraFiveM/kora-installer/security/advisories/new> (GitHub private vulnerability reporting).
2. Describe the problem, the affected version (`kora --version`) and how to reproduce it.

We aim to acknowledge a report within 7 days and to tell you what we plan to do within 30 days. Please give us a
reasonable time to release a fix before you disclose publicly.

## Kora Panel security model (0.4.0)

`kora panel` runs a small web server on your computer. In brief:

- It binds to `127.0.0.1` (default port 47120), so other computers cannot connect.
- A random token, new at each start, is printed in a sign-in link and exchanged for an `HttpOnly; SameSite=Strict`
  session cookie. Wrong tokens are rate-limited. `Authorization: Bearer <token>` is also accepted for scripts.
- Every request must carry the panel's own address in `Host` (DNS rebinding). Changing requests must also have a
  matching `Origin` (when sent) and an `X-Kora` header (CSRF).
- Responses carry a Content-Security-Policy that allows only the panel's own files (no external origins, no framing),
  `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`.
- Licence keys, database passwords and webhook addresses are accepted but never returned. Saved licence keys live in
  `secrets.json` in the Kora folder (plain text, readable only by you: `0600` from creation on Linux, inherited permissions
  removed on Windows, atomic writes, never replaced when unreadable); the API shows only a key's name and last four characters.
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
- Hub nodes authenticate with bearer tokens; treat them like passwords.
- Kora's own Git (MinGit, Windows) and GitHub CLI are downloaded only from `https://github.com/` release assets at a version
  and SHA-256 pinned in the source (`crates/kora-core/src/tools/pins.rs`, updated with `scripts/update-tool-pins.sh`).
  The file is verified before anything is unpacked, unpacked without links or paths that leave the folder, and moved into
  place in one step. They live in the user's own Kora folder and need no administrator rights; a Git or GitHub CLI already on
  the computer is preferred, and the folder Kora itself runs from is never searched for them.
- Switching a Development resource to another branch or a pull request (`kora branch`, `kora pr`, the GitHub tab) runs `git` and `gh` as the installing user, only inside the server's `resources` folder (a folder that points elsewhere is refused), and only with checked values: branch names and pull request numbers are validated before they reach a command line, and an address is only accepted when it is the resource's own repository or your fork. It never pushes, never forces, never deletes a branch or a stash, and never touches changes you have not saved without your explicit choice. A pull request is other people's code that then runs on your server: trying one needs the admin role, like other changes to the server (adding resources, rolling back). Production servers cannot be switched.

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
