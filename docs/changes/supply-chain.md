Changelog lines for the supply-chain / download safety work (branch `fix/supply-chain`), to be merged
into `CHANGELOG.md` under `[Unreleased]`.

### Added
- **Release files can be checked against the build that made them.** Every release file (`kora-linux-x86_64`, `kora-windows-x86_64.exe`, `install.sh`, `install.ps1`) now has a signed build provenance attestation from GitHub Actions. Check one with `gh attestation verify kora-linux-x86_64 --repo KoraFiveM/kora-installer`; see [CODE_SIGNING.md](CODE_SIGNING.md).
- `KORA_PRERELEASE=1` (Windows: `-Prerelease`) lets the install scripts take a pre-release.

### Changed
- **The install scripts install the newest stable release.** Pre-releases and drafts are skipped unless you ask for one (`KORA_VERSION=vX.Y.Z` / `-Version`, or `KORA_PRERELEASE=1`).
- **`kora self-update` stays on stable releases.** A stable build only updates to stable releases (and the "new version" notice follows the same rule); a pre-release build still gets the next pre-release and then the final one. `--version` installs any release, pre-releases included.
- **Downloads stay on https.** `install.sh` refuses a redirect from https to http (curl and wget), and `install.ps1` refuses a download that ended up on plain http. `install.sh` now runs as a single function called on its last line, so a `curl | sh` download that is cut short runs nothing.
- Every GitHub Action in the build and release workflows is pinned to a full commit SHA, and CI fails if one is not.

### Fixed
- **FXServer downloads can no longer write outside their folder through symlinks.** A link inside the Linux archive could point outside the folder and a later entry be written through it (for example `alpine/a -> /` and then `alpine/a/etc/...`, or chains of relative links). Kora now never creates a file, folder or link through a symlink, refuses anything placed beneath a link, and refuses links whose `..` steps pass through another link. The real FXServer archive extracts as before.
- Resources installed from a subfolder of a repository (`subpath`) refuse a subfolder that is a symlink or reached through one, so a repository cannot make Kora move a folder from elsewhere on the computer.
- Catalog paths with a drive letter or `:` (for example `C:/x`) are refused, and every clone or copy is checked to land inside the server's `resources` folder (this mattered on Windows).
- Resource and paid asset names `.`, `..` (any all-dots name) and names starting with `-` are refused.
- `kora self-update` no longer carries setuid/setgid bits from the old program file over to the new one.
- Files that must be private (the panel's nodes file, files written by the service setup with a mode, the app install's files) can no longer keep looser permissions from a file left behind by an earlier run.
