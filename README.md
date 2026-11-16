# Kora Installer

Kora Installer (`kora`) sets up and runs a [Kora](https://koraframe.work) FiveM server for you on Windows or Linux. One
guided wizard installs the server files, database, resources and services, and starts the server.

Documentation: <https://docs.koraframe.work/guides/installer/overview>

> **Status: pre-release.** It works, but expect rough edges, especially on Windows. See [Status and limits](#status-and-limits).

## Kora Panel

New in 0.4.0: run `kora panel` and manage your servers in the browser. It installs servers (the same wizard as
`kora install`), starts, stops and updates them, shows the console and logs, and handles resources, schedules and
notifications. It listens on `127.0.0.1` only (port 47120, or the next free one up to 47140 when something else uses it; `--port` changes it) and signs you in with a one-time link.
On a server without a screen, `kora panel --no-open` prints the link and an `ssh -L` tunnel command.

Guide: [docs/panel.md](docs/panel.md). The terminal wizard (`kora install`) and manager (`kora manager`) are unchanged.

## Install

Linux (Debian 12+ / Ubuntu 22.04+):

```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/download/v0.4.3/install.sh | sh
```

Windows 10/11 or Server 2019+ (PowerShell):

```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/download/v0.4.3/install.ps1 | iex
```

When the script finishes, Kora Panel starts by itself and opens in your browser. It runs in the background
(a systemd user service on Linux, a sign-in entry on Windows), so it keeps watching your servers and comes back after
a restart. The sign-in link is always printed too. On a server with no screen (over SSH) nothing opens: the script
prints the link and an `ssh -L` command to run on your own computer first. Show the link again with
`kora panel --print-link`; stop and remove the background panel with `kora panel --remove-service`. If the panel
cannot start (for example under `sudo` without a user session), the install still finishes and tells you to run
`kora panel` yourself. To only install the program, set `KORA_NO_LAUNCH=1` (Linux: `curl ... | KORA_NO_LAUNCH=1 sh`;
Windows: `$env:KORA_NO_LAUNCH=1` before `irm ... | iex`, or `-NoLaunch`). Prefer the terminal? `kora install` is the
terminal wizard.

The scripts check the download's SHA-256, install `kora` for your user (no administrator rights
needed for this step) and keep the previous version as `kora.previous`. Use the newest version number from the
[Releases page](https://github.com/KoraFiveM/kora-installer/releases) (`/releases/latest` ignores pre-releases, so use an
explicit version). `KORA_VERSION=vX.Y.Z` (or `-Version` on Windows) picks a release, `KORA_RELEASE_BASE` changes the download address.

## What the wizard does

1. **Welcome:** choose the guided or advanced questions, then production or development.
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
`kora panel` (web panel), `kora uninstall [--dry-run]` (asks item by item before removing anything Kora created; your folders and databases are kept unless you say otherwise).

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
| `crates/kora-tui` | the ratatui wizard and screens |
| `crates/kora-web` | the Kora Panel web server (axum) and the embedded UI |
| `crates/kora-cli` | the `kora` binary (command line) |
| `web/` | the panel's browser UI (Vite, React, TypeScript, Tailwind); built in CI and embedded |

The CLI, the terminal screens and the panel all reach the engine through one shared interface (`kora_core::api`), so they cannot drift apart.

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

The Windows `.exe` is **not code-signed**; SmartScreen may warn. The install scripts verify the SHA-256 of every download, and you can check it by hand against the `.sha256` file (see above). Details: [CODE_SIGNING.md](CODE_SIGNING.md).

- **Privacy:** Kora contacts nothing unless you ask it to: GitHub releases (self-update), the public resource catalog, runtime.fivem.net (FXServer downloads) and GitHub through `gh` (development mode forks).

## License

Licensed under the [Apache License 2.0](LICENSE).
