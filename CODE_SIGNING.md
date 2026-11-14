# Code signing policy

> Signing is pending: the SignPath Foundation application is made after the first public
> (unsigned) release. Releases are unsigned until it is approved.

Free code signing provided by SignPath.io, certificate by SignPath Foundation

## Roles

All members must use multi-factor authentication on GitHub and SignPath.

| Role | Responsibility | Members |
|---|---|---|
| Authors | Write code that is built and signed | The Kora maintainers ([KoraFiveM](https://github.com/orgs/KoraFiveM/people)) |
| Reviewers | Review changes before merge | The Kora maintainers ([KoraFiveM](https://github.com/orgs/KoraFiveM/people)) |
| Approvers | Manually approve every release signing request | The Kora maintainers ([KoraFiveM](https://github.com/orgs/KoraFiveM/people)) |

## Scope

- Only Kora's own binaries, built from source in CI from this public repository. No third-party binaries are signed.
- Every release signing request requires manual approval by an approver.
- Kora changes your system (services, scheduled tasks, firewall rules, registry entry). The installer lists every change before it happens, and `kora uninstall` removes them.

## Privacy policy

Privacy policy: <https://koraframe.work/privacy>. Usage statistics and error reports are
opt-in, described in the policy, shown during install, and can be disabled at any time.

## Build and metadata

Windows file metadata (product name "Kora", version) is set from the workspace version via
`crates/kora-cli/build.rs`. No packers (e.g. UPX) are used.
