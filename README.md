# Kora Installer

Kora Installer (`kora`) sets up and runs a [Kora](https://koraframe.work) FiveM server for you on Windows or Linux. One
guided wizard installs the server files, database, resources and services, and starts the server.

Documentation: <https://docs.koraframe.work/guides/installer/overview>

> **Status: pre-release.** It works, but expect rough edges, especially on Windows. See [Status and limits](#status-and-limits).

## Install

Linux (Debian 12+ / Ubuntu 22.04+):

```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/download/v0.3.3/install.sh | sh
```

Windows 10/11 or Server 2019+ (PowerShell):

```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/download/v0.3.3/install.ps1 | iex
```

Then run `kora install`. The scripts check the download's SHA-256, install `kora` for your user (no administrator rights
needed for this step) and keep the previous version as `kora.previous`. Use the newest version number from the
[Releases page](https://github.com/KoraFiveM/kora-installer/releases) (`/releases/latest` ignores pre-releases, so use an
explicit version). `KORA_VERSION=vX.Y.Z` (or `-Version` on Windows) picks a release, `KORA_RELEASE_BASE` changes the download address.

## What the wizard does

1. **Welcome:** choose production or development, and optionally paste a setup code from the website.
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

### Setup codes

Generate a code at <https://koraframe.work/installer> and paste it in the Welcome step (or `kora install --code <code>`).
The installer contacts koraframe.work to redeem it, see the [privacy policy](https://koraframe.work/privacy).

## Requirements

- Windows 10/11 or Server 2019+, or Debian 12+ / Ubuntu 22.04+ (x86-64).
- Administrator rights: Kora asks for them itself (a UAC prompt, or `sudo`).
- A few GiB of free disk (the installer checks before it starts) and internet access.
- Free ports: game port 30120 and txAdmin 40120 by default (the wizard suggests others if they are taken), database 3306.
- A Cfx.re licence key.

## Everyday commands

`kora status`, `kora start|stop|restart`, `kora schedule show|set|off <server>`, `kora delete <server>`,
`kora uninstall [--dry-run]` (asks item by item before removing anything Kora created; your folders and databases are kept unless you say otherwise).

## Status and limits

- Linux is tested end to end. Windows is covered by CI smoke tests; real-machine runs have started, so please report problems.
- Not released yet (see [RELEASE_NOTES.md](RELEASE_NOTES.md) and [CHANGELOG.md](CHANGELOG.md)): the `kora manager`
  screen, the Start Menu / app install, and automatic updates for production servers. Check the changelog for what is newer than the version you run.
- **The Windows `.exe` is unsigned** until the SignPath application is approved, so SmartScreen warns ("More info", "Run anyway").
  The install scripts verify checksums for you. To check by hand, compare with the `.sha256` file next to each download:

```sh
sha256sum -c kora-linux-x86_64.sha256
```
```powershell
(Get-FileHash kora-windows-x86_64.exe -Algorithm SHA256).Hash.ToLower()
```

## Build from source

Needs a stable Rust toolchain (1.75+).

```sh
cargo build --release -p kora-cli     # target/release/kora
```

## Development

| Crate | Purpose |
|---|---|
| `crates/kora-core` | the engine: step runner, installer, catalog, git, database, artifacts, txAdmin, services, firewall, updates |
| `crates/kora-tui` | the ratatui wizard and screens |
| `crates/kora-cli` | the `kora` binary (command line) |

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

- CI (`ci.yml`) builds and tests on Linux and Windows. `windows-smoke.yml` runs real Windows checks (`scripts/smoke/`),
  also on demand through *Run workflow*. `supply-chain.yml` runs `cargo deny` (advisories, licenses, sources).
- **Releases:** `scripts/bump-version.sh X.Y.Z` sets the version, commit it, tag `vX.Y.Z` and push the tag; the Release
  workflow builds the binaries from source, checks them and creates a draft release with the `.sha256` files and install scripts.

See [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md) and the [Code of Conduct](CODE_OF_CONDUCT.md).

## Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).

Windows binaries are signed only when they are built from this repository by the public GitHub Actions workflow, and only after a maintainer manually approves each release. Until the SignPath Foundation application is approved, releases are **unsigned** and Windows SmartScreen may warn; verify downloads with the published `.sha256` files. See [CODE_SIGNING.md](CODE_SIGNING.md) for the full policy.

- **Committers and reviewers:** the Kora maintainers ([KoraFiveM members](https://github.com/orgs/KoraFiveM/people))
- **Approvers:** the Kora maintainers (every signing request is approved by hand)
- **Privacy policy:** this program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it. When you use a website setup code, the installer contacts koraframe.work to redeem it; see <https://koraframe.work/privacy>.

## License

Licensed under the [Apache License 2.0](LICENSE).
