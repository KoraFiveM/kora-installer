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

## Code signing policy

Free code signing provided by SignPath.io, certificate by SignPath Foundation (pending; see [CODE_SIGNING.md](CODE_SIGNING.md)).

## License

TODO: owner must choose an OSI-approved license. See [LICENSE](LICENSE).
