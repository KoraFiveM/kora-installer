# Code signing policy (DRAFT)

> Draft for the SignPath Foundation application. Apply only after the first public
> (unsigned) release. Fill every TODO before applying.

Free code signing provided by SignPath.io, certificate by SignPath Foundation

## Roles

All members must use multi-factor authentication on GitHub and SignPath.

| Role | Responsibility | Members |
|---|---|---|
| Authors | Write code that is built and signed | TODO: GitHub handles |
| Reviewers | Review changes before merge | TODO: GitHub handles |
| Approvers | Manually approve every release signing request | TODO: GitHub handles |

## Scope

- Only Kora's own binaries, built from source in CI from this public repository. No third-party binaries are signed.
- Every release signing request requires manual approval by an approver.
- Kora changes your system (services, scheduled tasks, firewall rules, registry entry). The installer lists every change before it happens, and `kora uninstall` removes them.

## Privacy policy

TODO: link to the privacy policy (koraframe.work). Usage statistics and error reports are
opt-in, described in the policy, shown during install, and can be disabled at any time.

## Build and metadata

Windows file metadata (product name "Kora", version) is set from the workspace version via
`crates/kora-cli/build.rs`. No packers (e.g. UPX) are used.
