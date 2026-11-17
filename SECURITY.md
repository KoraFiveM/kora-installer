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
- Licence keys, database passwords and webhook addresses are accepted but never returned.
- The panel runs as the installing user, never as administrator: `kora panel` refuses to start elevated (set `KORA_ALLOW_ROOT_PANEL=1` only where root is the sole account). A bare `kora` (also `kora manager` and `kora install` with no options) never elevates: under `sudo` it runs again as the person who typed it, and from an administrator window it only opens a panel that already runs. Privileged actions are a command shown to you, or a
  helper (`kora helper`, Linux) that accepts only Kora's fixed list of privileged actions and checks the calling user.
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

Full user-facing description: [docs/panel.md](docs/panel.md). Report a panel vulnerability privately, in the same way
as any other (see above). Include the version, your platform, and whether the panel was local or remote.

## Supported versions

Kora Installer is pre-release software. Only the newest release receives security fixes.

## In scope

- **Privilege elevation:** UAC / `sudo` handling, anything that runs with more rights than needed, services and scheduled tasks.
- **Secrets handling:** the txAdmin password, database passwords, licence key and tokens (files, permissions, logs, clipboard).
- **Download verification:** SHA-256 checks and HTTPS enforcement in `kora` and in `install.sh` / `install.ps1`, and the release artifacts.
- **The panel:** authentication, session handling, the `Host` / `Origin` / `X-Kora` checks, the Content-Security-Policy, anything that lets a web page or another local user act on your servers, and anything that returns a secret to the browser.
- Overwriting or deleting files the installer should not touch.

## Out of scope

- Problems in third-party software that Kora installs (FXServer, txAdmin, MariaDB, git); report those upstream.
- The unsigned Windows executable triggering SmartScreen (known; see the README and CODE_SIGNING.md).
- Issues that need an attacker who already has administrator/root rights on the machine.
- Exposing the panel to an untrusted network with `--bind` over plain HTTP against the documented advice.
