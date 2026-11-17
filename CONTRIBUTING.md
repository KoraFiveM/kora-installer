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
  A new action is a variant of `Action` in Rust (its TypeScript is generated, see below); the page needs a label and a
  handler for it in `web/src/lib/problemActions.tsx`. The generated union includes `unknown`, and a page ignores a
  `kind` it has no handler for, so an older panel survives a newer Kora.
- **Words:** plain, friendly British spelling ("licence"), no jargon in what a person reads.
- **Changing `kora --help`:** the command help is checked against `crates/kora-cli/tests/golden/help.txt`; the test
  failure says how to refresh it.
- **Small commits** with a clear message; one topic per pull request. Add a line under `[Unreleased]` in `CHANGELOG.md`
  for user-visible changes.
- Install scripts: HTTPS only, always verify SHA-256, never overwrite a file that is not a kora program.

## Tests never touch your real Kora folder

`kora_home()` (`installer/registry.rs`) panics in a test binary that has not chosen a folder, and says what to use: call
`kora_core::installer::registry::use_test_home()` (an empty temporary folder for the test process), or set `KORA_HOME` to a
`tempfile::tempdir()` path and keep the `TempDir` alive, or give the `::at(path)` constructor of the store an explicit path. (The unit
tests of `kora-core` itself get the temporary folder automatically.) `KORA_ALLOW_REAL_HOME=1` switches the guard off for a deliberate
run. It guards the Kora folder only; a test that reads `HOME` directly is still on its own.

## Adding a command

`crates/kora-cli/src/main.rs` holds only the top-level `Cli` and `Command` enums, the process-level
concerns (which commands need administrator rights, relaunching, pausing the window) and the dispatch
`match`. Everything a command does lives in its own file under `crates/kora-cli/src/cmd/`
(shared prompts and printing helpers are in `ui.rs`). To add a command:

1. Create `cmd/<area>.rs` (and add `pub(crate) mod <area>;` to `cmd/mod.rs`). Put its clap definitions
   (`Args` struct or `Subcommand` enum), its handler (`pub(crate) fn run(..) -> i32`) and its tests there.
   Keep items private unless `main.rs` or another module needs them; use `pub(crate)`, never `pub`.
2. In `main.rs`, add one variant to `Command` (with its `///` help text), one arm in `command_kind` (does it
   need administrator rights?), and one dispatch arm in `main()`. If it needs root on Linux, add it to
   `admin_reasons_for_command`.
3. Add every command and flag to the table in `crates/kora-cli/src/parity.rs`: the panel operation that serves it
   (`Route(Op::..)`), or `CliOnly("why")`. `every_cli_item_is_classified` fails until you do.
4. Regenerate the help golden and read the diff: `KORA_UPDATE_GOLDEN=1 cargo test -p kora-cli help_tree_matches_golden`
   (it rewrites `crates/kora-cli/tests/golden/help.txt`).
5. Run `cargo fmt --all`, `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`.

Do not add handlers to `main.rs`. A file that grows past roughly 800 lines is a sign to split the area.

## Adding or changing an API operation (one place per layer, the TypeScript is generated)

The panel's HTTP API is described once, in Rust; `web/src/api/generated/` (checked in, marked "generated, do not
edit") is produced from that description and CI fails when it is out of date.

1. **`Op` in `crates/kora-core/src/api/ops.rs`**: the variant, `ALL`, `name`, `route` (method and path; `{s}`-style
   path parameters) and `is_job`. The router, `/meta` and the CLI help list follow from it.
2. **`spec(op)` in `crates/kora-core/src/api/spec.rs`**: what goes in and out: `OpSpec::get::<Response>()`,
   `OpSpec::call::<Request, Response>()`, `OpSpec::start_job_with::<Request>().events::<E>().result::<R>()`,
   `.with_query::<Q>()`, ... There is no wildcard arm: the code does not pass `cargo test` until the operation is
   listed. `OpSpec::untyped()` is an honest placeholder (the generated client says `Untyped`, the generator lists it);
   the `nothing_is_left_untyped` test keeps that list empty, so a new operation needs real types.
3. **The types**: a struct or enum in `crates/kora-core/src/api/wire/` (a file per area; new area, new file) that derives
   `Serialize`, `Deserialize` and, behind the `ts` feature, `ts_rs::TS`:
   `#[cfg_attr(feature = "ts", derive(ts_rs::TS))]`. A field that is left out when empty is
   `#[serde(default, skip_serializing_if = "Option::is_none")]` plus `#[cfg_attr(feature = "ts", ts(optional))]`; a
   field sent as `null` is a plain `Option`. An engine type that already has the wire shape can derive `TS` where it
   is (rename it with `ts(rename = "...")` if the name is taken). The handler should build and read those same types
   (see `handlers/servers.rs`); where it still uses `json!`, the contract test below is what keeps the two together.
4. **The handler and the route** (`handlers/*.rs`, `routes.rs`), as before; and `roles.rs` (`min_role`, `destructive`,
   `installs`, `local_only`).
5. **A case in `crates/kora-web/tests/api_contract.rs`**: it calls the real route with the fake backend and checks that
   the request body reads as the declared request type and that the answer reads as the declared response type *and
   writes back to the same JSON* (a field the type does not know, or `null` where the type leaves it out, fails).
   Job events and results are checked the same way. An operation that cannot run offline goes in `UNVERIFIED` with the
   reason.
6. **Regenerate**: `KORA_UPDATE_TS=1 cargo test -p kora-web --test api_ts`, then commit the files under
   `web/src/api/generated/`. The web client calls an operation by name (`call('server_get', { params: { s } })` in
   `web/src/api/client.ts`: path, method, body, query and answer all come from the generated `OpTypes`); add the
   one-line wrapper to `api` there. The mock backend (`web/src/api/mock/`) is type-checked against the same types, and
   `mock/parity.test.ts` fails when it lacks a route the Rust side has, so a mock that drifts fails `tsc` or that test.
7. Both parity tests (`crates/kora-cli/src/parity.rs`, `crates/kora-web/tests/parity.rs`) as before.

`web/src/api/types.ts` and `client.ts` are the web's stable import surface: `types.ts` re-exports the generated types
(and keeps a few deliberately looser readers, each with a comment); put a *new* type in Rust, not in `types.ts`.

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
