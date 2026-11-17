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

CI runs build, tests and clippy on Linux and Windows, lint, tests, build and bundle-size check of the UI, the browser tests
(see below), `shellcheck` and the tests for
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

## Browser tests (Playwright)

The panel UI is also tested in a real browser: `web/e2e/`, run by the **Browser tests** workflow (`e2e.yml`). Two layers:

- **`web/e2e/mock/`**: the UI built with the mock backend (`VITE_MOCK=1`, see `web/README.md`) and served by `vite preview`.
  Fast and deterministic. Journeys: first run and the wizard, a full Production and Development install, a failing step with
  retry / skip / change answers, the fixes that carry on by themselves, the Windows permission prompt, saved licence keys,
  the System check, every server tab, branches and pull requests, updates, Settings, what a remote operator or token admin
  sees, two keyboard-only journeys, and the "page file is gone after an update" reload screen. Every test also **fails on a
  browser console error, an unhandled rejection or a failed request**, and runs an **axe accessibility scan** on the pages it
  reaches (serious and critical violations fail the test).
- **`web/e2e/real/`**: a small smoke layer against the real `kora panel` binary (Linux in CI): a throw-away `HOME` and
  `KORA_HOME`, a spare port, a one-time sign-in link, the main pages, adding a fake server folder and opening its tabs. No
  installs.

```sh
cd web
npm ci --ignore-scripts
npx playwright install chromium              # once; a separate download, never part of `npm ci` (add --with-deps on a fresh Linux box)
npm run e2e                                  # the mock layer, Chromium (builds the mock UI into dist-mock/ and serves it)
npx playwright test e2e/mock/wizard.spec.ts  # one file; add -g "text" for one test, --headed to watch, --ui for the inspector
npm run e2e:real                             # the real-panel layer: needs a built kora (cargo build -p kora-cli, after `npm run build` so the UI is embedded) or KORA_BIN=/path/to/kora
npm run e2e:all                              # the mock layer in Chromium and the core journey in WebKit
npx playwright show-trace e2e-results/<test>/trace.zip   # after a failure (CI uploads these as an artifact)
```

Writing a test:

- Import `test` and `expect` from `e2e/fixtures.ts` (mock layer) or `e2e/real/panel.ts` (real layer): they add the
  console/request watcher. If a journey provokes an error on purpose, say so with `problems.expect(/text/)`.
- Open a page with `open('/path', { scenario, as })`: the scenario is a mock scenario (`default`, `empty`, `fleet`, `trouble`,
  `windows`, ...), `as` is who is looking (`operator`, `remote-operator`, `remote-admin`). The special install names of the
  mock (`fail-`, `skip-`, `github-`, `nogh-`, `nodb-`, `decline-`, `nobody-`, `remote-`) are described in `web/README.md`.
  The mock's world is rebuilt on every page load, so tests are independent; never rely on state from another test.
- Locate things like a person does: `getByRole`, `getByLabel`, `getByText` (always with `exact: true` when a name is also a
  prefix of another). No CSS selectors, no `waitForTimeout`: use `expect(...).toBeVisible()` and friends, which wait. A dialog
  hides the page behind it from the accessibility tree: use `includeHidden: true` to look at the page below.
- Call `await scan('what this page is')` on each new screen: it fails on serious or critical accessibility violations. Set
  `E2E_AXE_REPORT=1` to print the moderate ones as well. Fix a violation in the component when the fix is small; do not
  silence it in the test.
- Keyboard-only journeys use `tabTo(page, locator)` (Tab until that control has focus), `keyboard.press` and never `click`.

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
