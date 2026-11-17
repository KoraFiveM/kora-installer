# Kora Installer

Kora Installer (`kora`) sets up and runs a [Kora](https://koraframe.work) FiveM server for you on Windows or Linux. Its
web panel opens in your browser and walks you through installing the server files, database, resources and services,
then starts the server and lets you manage it.

Documentation: <https://docs.koraframe.work/guides/installer/overview>

> **Status: pre-release.** It works, but expect rough edges, especially on Windows. See [Status and limits](#status-and-limits).

## Kora Panel

The panel is Kora's only interface, and it is what "the Kora app" is: a page in your browser, plus a Start Menu / desktop
shortcut (a launcher in the applications menu on Linux) that opens it. Run `kora` (or double-click the shortcut): Kora
makes sure the panel is running in the background and opens your browser at it. With no servers yet it opens straight into
the install wizard. `kora` never elevates, and the panel runs as you. The parts that need administrator rights (services,
firewall rules, installing MariaDB, updating Kora) are done by Kora's small administrator service, which the installer
(`.msi` on Windows, `.deb` on Debian/Ubuntu, `sudo kora service install` elsewhere) sets up with one approval; without it
the panel offers to install it, to carry on without those parts, or shows the command to run.

The panel installs servers, starts, stops and updates them, shows the console and logs, and handles resources, schedules
and notifications. It listens on `127.0.0.1` only (port 47120, or the next free one up to 47140; `--port` changes it) and
signs you in with a one-time link.
On a server without a screen (over SSH), `kora` prints the sign-in link and an `ssh -L` tunnel command to run on your own
computer first. `kora panel --tailscale` instead opens it to your own devices over [Tailscale](https://tailscale.com)
(you sign in with your Tailscale account: no link or tunnel).

`kora install` (with no options) opens the install page. (The old name `kora manager` still works and is the same as `kora`.)
`kora install` with options stays the scriptable, non-interactive installer. The terminal screens (TUI) of earlier
versions are gone: everything they did is in the panel.

**The desktop app (optional).** `kora-desktop` is a tray icon and a small native window that shows the same panel: the icon
shows at a glance whether your servers run, its menu starts, stops and restarts them, and you get a notification when one
stops. It is a thin client of the panel (nothing changes for the browser route, remote use or a server without a screen),
Windows first (WebView2; Linux with WebKitGTK second). See [docs/desktop-app.md](docs/desktop-app.md).

Guide: [docs/panel.md](docs/panel.md). Where Kora keeps its own data, backups and the way back to older versions: [docs/store.md](docs/store.md).

## Install

Linux (Debian 12+ / Ubuntu 22.04+):

```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/latest/download/install.sh | sh
```

Windows 10/11 or Server 2019+ (PowerShell):

```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/latest/download/install.ps1 | iex
```

Both scripts first offer the **system install** (asked once; answer n to skip): the `.msi` on Windows (one Windows
permission prompt), the `.deb` on Debian/Ubuntu (your password once, or `sudo kora service install` on other
systems). It puts Kora in a protected folder together with Kora's administrator service, so installing servers,
firewall rules and databases later needs no password and no terminal (`KORA_SYSTEM=1` / `-System` does it without
asking, `KORA_SYSTEM=0` / `-PerUser` installs just for you). The per-user install below needs no administrator rights;
the parts of a server that need them are then a button, a command or skipped. You can also download
`kora-windows-x86_64.msi` / `kora-linux-x86_64.deb` from the release page yourself.

The script also makes a Start Menu and Desktop shortcut on Windows (a launcher in the applications menu on Linux
desktops); they are yours only, no administrator rights, and open the panel. `KORA_NO_SHORTCUTS=1` (or `-NoShortcuts`)
skips them. When the script finishes, Kora Panel starts by itself and opens in your browser. It runs in the background
(a systemd user service on Linux, a sign-in entry on Windows), so it keeps watching your servers and comes back after
a restart. The sign-in link is always printed too. On a server with no screen (over SSH) nothing opens. If Tailscale runs there, the script
offers to open the panel to your own Tailscale devices (`KORA_TAILSCALE=1` says yes in advance); otherwise it prints
the link and an `ssh -L` command to run on your own computer first. Show the link again with
`kora panel --print-link`; restart the background panel with `kora panel restart`; stop and remove it with `kora panel --remove-service`. If the panel
cannot start (for example under `sudo` without a user session), the install still finishes and tells you to run
`kora` yourself. To only install the program, set `KORA_NO_LAUNCH=1` (Linux: `curl ... | KORA_NO_LAUNCH=1 sh`;
Windows: `$env:KORA_NO_LAUNCH=1` before `irm ... | iex`, or `-NoLaunch`). Open Kora again any time with `kora` or the shortcut.

The scripts download over https only, check the download's SHA-256, install `kora` for your user (no administrator rights
needed for this step) and keep the previous version as `kora.previous`. They install the newest stable release (never a
pre-release or a draft); see the [Releases page](https://github.com/KoraFiveM/kora-installer/releases).
`KORA_VERSION=vX.Y.Z` (or `-Version` on Windows) picks a release, pre-releases included; `KORA_PRERELEASE=1`
(or `-Prerelease`) takes the newest release even if it is a pre-release; `KORA_RELEASE_BASE` changes the download address.

## What the install wizard does

1. **Welcome:** choose the short guided questions or every step, then where the resources come from (plain clones, or your own copies on GitHub) and which versions to follow.
2. **Choices:** server name and folder, your Cfx.re licence key, ports, which resources to install.
3. **Checks:** free disk space and port conflicts (with next-free-port suggestions) before anything is changed.
4. **Install:** downloads and verifies FXServer, sets up the database, clones/installs resources, creates the service and firewall rules.
5. **Done:** the server is started and the last screen offers Open txAdmin, Copy connect address and Stop server.

If a step fails, the install stops there and the page offers **Try this step again** (or **Skip this step** when it is
safe: firewall, schedule, starting the server) instead of starting over. An install that was stopped, for example by
restarting the panel, can be carried on from the install page when it used a saved licence key. Scripts can use flags
instead of the wizard (`kora install --help`, `kora install --dry-run` to preview).

### Where the resources come from, and which versions

- **Plain clones** (no GitHub account needed) or **your own copies** (forks in your GitHub account or an organisation you belong to, so you can change them, push and open pull requests).
- **Released versions** (pinned, stable: you choose when to move to a newer release) or the **latest changes** (the main branch). The two are separate choices; clones default to released and your own copies to the latest. Older servers made as "production" or "development" read as clones + released and own copies + latest.
- Everything else (the resource page, branches and pull requests, updates, the automatic update, the GitHub work) is the same for every server.

## Requirements

- Windows 10/11 or Server 2019+, or Debian 12+ / Ubuntu 22.04+ (x86-64).
- Administrator rights are **not** needed to install Kora, open the panel or get started. Setting up a server's
  service, firewall rules or a database server does need them: on Linux the panel shows the `sudo` command to run (or does
  it for you once the optional helper is installed); on Windows an install from the panel asks once, when it starts, with
  a normal Windows permission prompt for all of its administrator parts. If you say no, you can ask again, carry on
  without those parts, or see the command to run.
- Server folder (suggested by default): Windows `C:\Kora\<name>`; Linux `~/kora/<name>` in your own home folder, also
  when Kora runs with `sudo`. Logged in as root (common on rented servers), it is `/home/kora/<name>`: Kora creates a
  normal `kora` user account that owns it. On Linux the background service runs as its own account, which cannot enter
  a private home folder; the wizard then offers to let it pass through (`chmod o+x` on exactly those folders) or to use
  `/home/kora/<name>`. Without the wizard use `kora install ... --fix-permissions` or `--dir`.
- A few GiB of free disk (the installer checks before it starts) and internet access.
- Free ports: game port 30120 and txAdmin 40120 by default (the wizard suggests others if they are taken), database 3306.
- A Cfx.re licence key.

## Everyday commands

Most things are in the panel (`kora`); these are the same operations for scripts and terminals. `kora --help` and
`kora <command> --help` list every option.

| Command | What it does |
|---|---|
| `kora` (also `kora manager`, `kora install` with no options) | Make sure the panel runs in the background and open it (the install page for `kora install`). |
| `kora panel` | The panel in this window, or set up and manage it: `--tailscale`, `--setup`, `--print-link`, `--new-token`, `--remove-service`, `restart`, `remote-actions on\|off\|status`, `user invite\|reset\|credentials\|list\|remove\|api-token ...` (remote users: passkeys and one-time codes), `tls status\|set\|self-signed\|tailscale\|off\|renew\|export-ca` (HTTPS), `proxy trust\|forget\|status` (a reverse proxy), `tailscale allow\|remove\|list\|off`. |
| `kora install --name ... [--source clone\|fork] [--versions released\|latest] [--fork-owner <login-or-org>] ...` | Install a server without asking (`--dry-run` to preview, `--license-key-name` to use a saved key). `--mode prod\|dev` is the old spelling of the first two and still works. |
| `kora status`, `kora start\|stop\|restart <server>` | List servers; control a server's background service. |
| `kora update <server>` | Preview, apply and roll back updates (`--auto` is for schedulers). `kora schedule show\|set\|off <server>` sets when a server updates itself. |
| `kora add-resources`, `kora add-asset <server> <zip>...` | Add catalogue resources, or paid assets downloaded from the Cfx.re Portal. |
| `kora branch list\|show\|switch\|default\|all\|restore <server> ...`, `kora pr list\|checkout\|view\|comment\|review\|merge <server> <resource> ...` | Change a resource's branch or try a pull request (updates leave a switched resource alone), and read, comment on, review or merge pull requests as the GitHub account `gh` is signed in as. |
| `kora resource show\|changes\|log\|commit\|push\|new-branch\|issues\|issue-comment\|releases\|fork\|versions <server> ...` | One resource: where it is and whether the running server needs a restart; what changed, commit, push, a new branch, issues, releases; make your own copy (fork); change which versions the server follows. |
| `kora keys list\|add\|replace\|rename\|remove\|use\|keychain status\|on\|off`, `kora profile show\|forget` | Saved licence keys (a key is read from `CFX_LICENSE_KEY` or standard input, never from the command line; kept in the system keychain when there is one: Windows DPAPI, Linux Secret Service) and the remembered Cfx.re name and GitHub owner. |
| `kora check [--start-database]`, `kora tools status\|install`, `kora github owners` | The System check, Kora's own Git and GitHub CLI copies, GitHub helpers. |
| `kora webhook ...` | Discord notifications for update reports. |
| `kora backup create\|list\|verify\|restore\|undo\|delete\|prune\|schedule <server>`, `kora backup import <file>` | Back up a server (database, settings, txAdmin data, resources; optional password), check a backup, restore it (a safety copy is made first and can be undone), set where backups go and a schedule, or bring a server from a backup (or another computer) as a new one. The file holds secrets: a password is read from `--password-stdin` or `KORA_BACKUP_PASSWORD`, never an argument. |
| `kora support bundle [<server>]` | One file to hand to whoever is helping you, with every secret removed. You see what is in it first; it is never uploaded. |
| `kora app install\|status\|uninstall` | Kora's per-user shortcuts (Start Menu / Desktop, applications menu). `kora uninstall --app` removes the OLD machine-wide app of earlier versions. |
| `kora self-update` | Update Kora to the newest stable release (checksum-verified). |
| `kora delete <server>` | Delete a server and what the installer created for it. |
| `kora uninstall [--dry-run]` | Remove the system changes Kora made. Asks item by item; your server folders and databases are kept unless you say otherwise, and so are your saved keys. |
| `kora helper install\|uninstall\|status` | The optional helper that lets the panel do the few things that need administrator rights. |

## Status and limits

- Linux is tested end to end. Windows is covered by CI smoke tests; real-machine runs have started, so please report problems.
- Automatic updates for servers on released versions are new in 0.8 and off until turned on. See [RELEASE_NOTES.md](RELEASE_NOTES.md) and
  [CHANGELOG.md](CHANGELOG.md); check the changelog for what is newer than the version you run.
- **The Windows `.exe` is unsigned**, so SmartScreen may warn ("More info", "Run anyway").
  The install scripts verify the SHA-256 checksum for you. To check by hand, compare with the `.sha256` file next to each download:

```sh
sha256sum -c kora-linux-x86_64.sha256
```
```powershell
(Get-FileHash kora-windows-x86_64.exe -Algorithm SHA256).Hash.ToLower()
```

## Building from source

Needs Rust 1.93 or newer (the minimum supported version) and, to include the panel's web UI, Node.js (see `web/.nvmrc`).

```sh
cd web && npm ci --ignore-scripts && npm run build && cd ..   # builds web/dist, embedded into the binary
cargo build --release -p kora-cli                              # target/release/kora
```

Without the UI step `cargo build` still works, but `kora panel` then shows a "UI not built" page. Release binaries are
built with the UI. See [web/README.md](web/README.md) for UI development (a mock backend is included).

## Development

| Crate | Purpose |
|---|---|
| `crates/kora-core` | the engine: step runner, installer, catalog, git, database, artifacts, txAdmin, services, firewall, updates |
| `crates/kora-web` | the Kora Panel web server (axum) and the embedded UI |
| `crates/kora-cli` | the `kora` binary (command line) |
| `web/` | the panel's browser UI (Vite, React, TypeScript, Tailwind); built in CI and embedded |

The CLI and the panel both reach the engine through one shared interface (`kora_core::api`), so they cannot drift apart.

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

- CI (`ci.yml`) builds and tests on Linux and Windows, and lints, tests and builds the panel UI. `windows-smoke.yml` runs real Windows checks (`scripts/smoke/`),
  also on demand through *Run workflow*. `supply-chain.yml` runs `cargo deny` (advisories, licenses, sources) and `npm audit` for the panel UI.
- **Releases:** `scripts/bump-version.sh X.Y.Z` sets the version, commit it, tag `vX.Y.Z` and push the tag; the Release
  workflow builds the binaries from source, checks them and creates a draft release with the `.sha256` files and install scripts.

See [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md) and the [Code of Conduct](CODE_OF_CONDUCT.md).

## Code signing and privacy

The Windows `.exe` is **not code-signed**; SmartScreen may warn. The install scripts verify the SHA-256 of every download, and you can check it by hand against the `.sha256` file (see above). Every release file also has a signed build provenance attestation, which proves it was built by this repository's release workflow; check it with the [GitHub CLI](https://cli.github.com/):

```sh
gh attestation verify kora-linux-x86_64 --repo KoraFiveM/kora-installer
```

Details: [CODE_SIGNING.md](CODE_SIGNING.md).

- **Privacy:** Kora contacts nothing unless you ask it to: GitHub releases (self-update), the public resource catalog, runtime.fivem.net (FXServer downloads) and GitHub through `gh` (development mode forks). Development mode needs Git and the GitHub CLI: if they are not on the computer Kora downloads its own copies (MinGit and `gh`) from the official GitHub releases when you press *Install it for me* (or during the install), verified against checksums pinned in the source, into its own folder, with no administrator rights (see the System check in [docs/panel.md](docs/panel.md#the-system-check)).

## License

Licensed under the [Apache License 2.0](LICENSE).
