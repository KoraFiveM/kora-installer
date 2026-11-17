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
  A new action is a variant of `Action` in Rust (its TypeScript is generated, see below); the page needs a label and a
  handler for it in `web/src/lib/problemActions.tsx`. The generated union includes `unknown`, and a page ignores a
  `kind` it has no handler for, so an older panel survives a newer Kora.
- **Words:** plain, friendly British spelling ("licence"), no jargon in what a person reads.
- **Changing `kora --help`:** the command help is checked against `crates/kora-cli/tests/golden/help.txt`; the test
  failure says how to refresh it.
- **Small commits** with a clear message; one topic per pull request. Add a line under `[Unreleased]` in `CHANGELOG.md`
  for user-visible changes.
- Install scripts: HTTPS only, always verify SHA-256, never overwrite a file that is not a kora program.

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
