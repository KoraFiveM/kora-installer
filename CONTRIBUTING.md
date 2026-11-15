# Contributing

Thanks for helping. Bug reports and pull requests are welcome.

## Build and test

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all
```

CI runs build, tests and clippy on Linux and Windows, plus `shellcheck` and the tests for `scripts/install.sh`, and a
supply-chain check (`cargo deny`). All must pass. Run `cargo deny check` locally if you add or update a dependency.

## Conventions

- **Testability:** system access (commands, files, network, services, firewall) goes through traits with fake
  implementations in tests. Unit tests must not change the machine they run on.
- **Platforms:** no unix-only code outside `#[cfg(unix)]` and no Windows-only code outside `#[cfg(windows)]`; the code must
  compile on both.
- **Line endings:** LF everywhere (`.gitattributes` enforces it), including `.ps1` files.
- **Secrets:** never log or print passwords, keys or tokens. Use the `Secret` type.
- **Small commits** with a clear message; one topic per pull request. Add a line under `[Unreleased]` in `CHANGELOG.md`
  for user-visible changes.
- Install scripts: HTTPS only, always verify SHA-256, never overwrite a file that is not a kora program.

## Windows smoke tests

Real Windows behaviour (services, firewall, MariaDB, install script) is tested by the **Windows smoke** workflow.
It runs on pushes to `main`; to run it on your branch: GitHub > Actions > *Windows smoke* > *Run workflow* > pick your
branch. The scripts live in `scripts/smoke/`.

## Pull requests

Open a PR against `main` and fill in the template. A maintainer reviews every change. By contributing you agree that
your work is licensed under the Apache License 2.0.

See also the [Code of Conduct](CODE_OF_CONDUCT.md) and the [security policy](SECURITY.md).
