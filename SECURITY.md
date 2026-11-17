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
- The panel runs as the installing user, never as administrator: `kora panel` refuses to start elevated (set `KORA_ALLOW_ROOT_PANEL=1` only where root is the sole account). A bare `kora` (also `kora manager` and `kora install` with no options) never elevates: under `sudo` it runs again as the person who typed it, and from an administrator window it only opens a panel that already runs. Privileged actions are a command shown to you, or a
  helper (`kora helper`, Linux) that accepts only Kora's fixed list of privileged actions and checks the calling user.
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
- A stopped install is kept in memory for six hours, with its licence key and database password. When it used a saved key it is also written to `paused-installs/` in the Kora folder for three days without any secret, so it can carry on after a restart.
- `kora panel` and a bare `kora` print a sign-in link with a one-time code. If the running panel cannot give one (an older version, or it does not answer), the link carries the panel's own token instead: do not paste that link into chats or tickets.

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
