# Kora installer and manager

One Rust binary, `kora`, that installs and manages Kora servers on Windows and Linux.

> Status: M0 scaffold. Most subcommands are stubs.

## Layout

- `crates/kora-core`: step engine (implemented), plus stubs for catalog, git, db, artifacts, txadmin, service, firewall, update
- `crates/kora-tui`: ratatui front end (welcome screen stub)
- `crates/kora-cli`: the `kora` binary (`install`, `manager`, `update --auto <server>`, `status`, `uninstall`)

## Build

```
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

## Get the program

Linux: `curl -fsSL <release-base>/install.sh | sh` downloads the latest release, checks its SHA-256 and installs it to
`/usr/local/bin` (root) or `~/.local/bin`. Windows: `irm <release-base>/install.ps1 | iex` installs to
`%LOCALAPPDATA%\Kora\bin` and adds it to your user PATH (no administrator rights). Both refuse non-HTTPS download
addresses (except localhost), never replace a file that is not a kora program, and keep the previous version as `kora.previous`.
`KORA_RELEASE_BASE` changes where they download from.

## Removing things

`kora delete <server>` removes one server (and the service account Kora created for it). `kora uninstall` asks, item by
item, before removing the system changes Kora made (services, update timers, firewall rules, service accounts, the
server list entry); server folders and databases are kept unless you say otherwise. `kora uninstall --dry-run` shows the plan.

## Code signing policy

Free code signing provided by SignPath.io, certificate by SignPath Foundation (pending; see [CODE_SIGNING.md](CODE_SIGNING.md)).

## Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).

Windows binaries are signed only when they are built from this repository by the public GitHub Actions workflow, and only after a maintainer manually approves each release. Until the SignPath Foundation application is approved, releases are **unsigned** and Windows SmartScreen may warn; verify downloads with the published `.sha256` files. See [CODE_SIGNING.md](CODE_SIGNING.md) for the full policy.

- **Committers and reviewers:** the Kora maintainers ([KoraFiveM members](https://github.com/orgs/KoraFiveM/people))
- **Approvers:** the Kora maintainers (every signing request is approved by hand)
- **Privacy policy:** this program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it. When you use a website setup code, the installer contacts koraframe.work to redeem it; see <https://koraframe.work/privacy>.

## License

Licensed under the [Apache License 2.0](LICENSE).
