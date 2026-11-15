## What and why

<!-- What does this change, and why? Link the issue if there is one. -->

## Checklist

- [ ] `cargo test --workspace --locked` passes
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` passes
- [ ] `cargo fmt --all` has been run
- [ ] New logic has tests (fake runners/traits, no real system changes in unit tests)
- [ ] No unix-only code outside `#[cfg(unix)]` / no Windows-only code outside `#[cfg(windows)]`
- [ ] No secrets in code, logs or test fixtures
- [ ] User-visible change: added a line under `[Unreleased]` in CHANGELOG.md
- [ ] Touches Windows behaviour: ran the "Windows smoke" workflow on this branch (Actions > Windows smoke > Run workflow)
