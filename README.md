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
the install wizard. Nothing asks for administrator rights; `kora` never elevates, and the panel runs as you. When an
action does need them (services, firewall rules, installing MariaDB) the panel shows the exact command to run as
administrator.

The panel installs servers, starts, stops and updates them, shows the console and logs, and handles resources, schedules
and notifications. It listens on `127.0.0.1` only (port 47120, or the next free one up to 47140; `--port` changes it) and
signs you in with a one-time link.
On a server without a screen (over SSH), `kora` prints the sign-in link and an `ssh -L` tunnel command to run on your own
computer first. `kora panel --tailscale` instead opens it to your own devices over [Tailscale](https://tailscale.com)
(you sign in with your Tailscale account: no link or tunnel).

`kora manager` and `kora install` (with no options) are the same as `kora`; `kora install` opens the install page.
`kora install` with options stays the scriptable, non-interactive installer. The terminal screens (TUI) of earlier
versions are gone: everything they did is in the panel.

Guide: [docs/panel.md](docs/panel.md).

## Install

Linux (Debian 12+ / Ubuntu 22.04+):

```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/latest/download/install.sh | sh
```

Windows 10/11 or Server 2019+ (PowerShell):

```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/latest/download/install.ps1 | iex
```

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

1. **Welcome:** choose the short guided questions or every step, then production or development.
2. **Choices:** server name and folder, your Cfx.re licence key, ports, which resources to install.
3. **Checks:** free disk space and port conflicts (with next-free-port suggestions) before anything is changed.
4. **Install:** downloads and verifies FXServer, sets up the database, clones/installs resources, creates the service and firewall rules.
5. **Done:** the server is started and the last screen offers Open txAdmin, Copy connect address and Stop server.

If an install is interrupted, run `kora install` again to resume it. Scripts can use flags instead of the wizard
(`kora install --help`, `kora install --dry-run` to preview).

### Production vs development mode

- **Production:** a server that runs for players. Resources are installed from releases; updating to newer releases is manual for now.
- **Development:** you sign in to GitHub in the wizard and pick where your forks go (your account or an organisation).
  The server updates itself on a schedule you choose (`kora schedule`).

## Requirements

- Windows 10/11 or Server 2019+, or Debian 12+ / Ubuntu 22.04+ (x86-64).
- Administrator rights: Kora asks for them itself (a UAC prompt, or `sudo`).
- Server folder (suggested by default): Windows `C:\Kora\<name>`; Linux `~/kora/<name>` in your own home folder, also
  when Kora runs with `sudo`. Logged in as root (common on rented servers), it is `/home/kora/<name>`: Kora creates a
  normal `kora` user account that owns it. On Linux the background service runs as its own account, which cannot enter
  a private home folder; the wizard then offers to let it pass through (`chmod o+x` on exactly those folders) or to use
  `/home/kora/<name>`. Without the wizard use `kora install ... --fix-permissions` or `--dir`.
- A few GiB of free disk (the installer checks before it starts) and internet access.
- Free ports: game port 30120 and txAdmin 40120 by default (the wizard suggests others if they are taken), database 3306.
- A Cfx.re licence key.

## Everyday commands

`kora status`, `kora start|stop|restart`, `kora schedule show|set|off <server>`, `kora delete <server>`,
`kora panel` (web panel), `kora branch list|show|switch|default|all|restore <server> ...` and `kora pr list|checkout <server> <resource> ...` (Development servers: change a resource's branch or try a pull request; updates leave a switched resource alone), `kora uninstall [--dry-run]` (asks item by item before removing anything Kora created; your folders and databases are kept unless you say otherwise).

## Status and limits

- Linux is tested end to end. Windows is covered by CI smoke tests; real-machine runs have started, so please report problems.
- Not released yet: automatic updates for production servers. See [RELEASE_NOTES.md](RELEASE_NOTES.md) and
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

- **Privacy:** Kora contacts nothing unless you ask it to: GitHub releases (self-update), the public resource catalog, runtime.fivem.net (FXServer downloads) and GitHub through `gh` (development mode forks).

## License

Licensed under the [Apache License 2.0](LICENSE).
