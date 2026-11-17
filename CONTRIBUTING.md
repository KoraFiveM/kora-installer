# Contributing

Thanks for helping. Bug reports and pull requests are welcome.

## Build and test

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all
```

The panel's browser UI is in `web/` (Vite, React, TypeScript): run `npm ci --ignore-scripts` once, then `npm run lint`,
`npm test` and `npm run build` there (a mock backend is included; see `web/README.md`).

CI runs build, tests and clippy on Linux and Windows, lint, tests and build of the UI, `shellcheck` and the tests for
`scripts/install.sh`, and a supply-chain check (`cargo deny`, `npm audit`). All must pass. Run `cargo deny check` locally
if you add or update a dependency.

## Conventions

- **Testability:** system access (commands, files, network, services, firewall) goes through traits with fake
  implementations in tests. Unit tests must not change the machine they run on.
- **Platforms:** no unix-only code outside `#[cfg(unix)]` and no Windows-only code outside `#[cfg(windows)]`; the code must
  compile on both.
- **Line endings:** LF everywhere (`.gitattributes` enforces it), including `.ps1` files.
- **Secrets:** never log or print passwords, keys or tokens. Use the `Secret` type.
- **One engine, two front ends:** the command line and the panel both go through `kora_core::api` (`Op` and the
  `Backend` trait). A new operation needs an entry in both parity tests (`crates/kora-cli/src/parity.rs`,
  `crates/kora-web/tests/parity.rs`), or a stated exemption, so the two cannot drift apart. There is no terminal UI.
- **Errors a person can act on** are a `Problem` (`crates/kora-core/src/problem.rs`): a stable code, plain words,
  optional details and a list of actions the panel turns into buttons (retry, sign in to GitHub, install a tool, ...).
  New actions also need the matching entry in `web/src/api/types.ts`, a label and a handler.
- **Words:** plain, friendly British spelling ("licence"), no jargon in what a person reads.
- **Changing `kora --help`:** the command help is checked against `crates/kora-cli/tests/golden/help.txt`; the test
  failure says how to refresh it.
- **Small commits** with a clear message; one topic per pull request. Add a line under `[Unreleased]` in `CHANGELOG.md`
  for user-visible changes.
- Install scripts: HTTPS only, always verify SHA-256, never overwrite a file that is not a kora program.

## Windows smoke tests

Real Windows behaviour (services, firewall, MariaDB, install script) is tested by the **Windows smoke** workflow.
It runs on pushes to `main`; to run it on your branch: GitHub > Actions > *Windows smoke* > *Run workflow* > pick your
branch. The scripts live in `scripts/smoke/` and are numbered in the order the workflow runs them. The Linux
counterpart (`linux-smoke.yml`, scripts in `scripts/smoke-linux/`) runs the same way. Before a release the maintainer also
walks through [docs/windows-test-checklist.md](docs/windows-test-checklist.md) on a real Windows machine.

## Pull requests

Open a PR against `main` and fill in the template. A maintainer reviews every change. By contributing you agree that
your work is licensed under the Apache License 2.0.

See also the [Code of Conduct](CODE_OF_CONDUCT.md) and the [security policy](SECURITY.md).
