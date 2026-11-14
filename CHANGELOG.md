# Changelog

All notable changes to Kora Installer. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [SemVer](https://semver.org/).

## [Unreleased]

### Added
- The installer now starts your server when it finishes (final step "Start your server"): FXServer runs in the background (or through its service), Kora waits until txAdmin answers and tells you plainly if it does not come up (rejected licence key, port clash, crash, with the last log lines). The final screen has [ Open txAdmin ], [ Copy connect address ], [ Copy password ], [ Open folder ] and [ Stop server ] (or [ Start server ] when it is not running). `kora install --start` / `--no-start` for scripts (off by default there); `kora start`/`stop` work without a service; `kora status` shows the running state and port states.
- Development mode: the installer signs you in to GitHub (one-time code, copy and open buttons) and lets you pick where your forks go - your own account or one of your organisations (greyed out with the reason when one can't be used) - or create a new organisation. Your last choice is remembered.

### Changed
- The generated txAdmin password is no longer printed in the install notes or log; it is in `secrets/txadmin-admin.txt` and shown on the final screen.
- `kora status`/`start`/`stop` no longer treat a server without a systemd service as having a stopped one.
- Kora now asks for administrator permission itself (Windows UAC prompt, or `sudo`) instead of telling you to "Run as administrator". On Windows, interactive commands (`kora`, `install`, `delete`, `uninstall`, `start`/`stop`/`restart`, `update`) reopen themselves elevated; `--version`, `--help`, `status`, dry runs and non-interactive use stay unelevated. `--no-elevate` turns this off.

## [0.2.0] - 2026-11-14

First broadly usable build. Tested on Linux; the Windows build is CI-tested and real Windows runs
have started, so expect rough edges there. See `RELEASE_NOTES.md` for known gaps.

### Added
- Setup code: redeem a code from the website in the wizard's Welcome stage or with `kora install --code`.
- `kora uninstall` with per-item prompts, `--dry-run` and `--yes`; the safe defaults keep your folders and databases.
- Dedicated service account for the server service, plus a root-owned copy of `kora` for services.
- Free-space preflight before installing.
- Catalog `dependsOn`: selecting a resource also selects its dependencies (for example kora-lib -> kora-theme).
- `kora install --dry-run` prints the effective choices (name, mode, resources, ports, artifact) before the plan.
- Port handling: conflict check in prepare, re-check before install, "next free port" suggestions,
  `kora install --auto-ports`, live port status in the wizard.
- Wizard: clipboard Paste/Clear buttons and bracketed paste; clickable log and folder actions on the failure and outcome screens.
- Install scripts `install.sh` (Linux) and `install.ps1` (Windows): HTTPS-only, SHA-256 verified, never overwrite an
  unknown file; `KORA_VERSION` / `-Version` to pick a release. Shipped as release assets.
- Windows: MariaDB installed end to end (download, verify, initialise, service, admin file) with uninstall;
  restricted ACL on `secrets.cfg`; elevation check.
- Windows smoke-test workflow (install script, production install, dev dry-run, service and firewall steps).
- Release tooling: rehearsable release workflow (asset, version and checksum checks), `scripts/bump-version.sh`.
- Apache-2.0 license and a code signing policy (`CODE_SIGNING.md`).

### Changed
- Install scripts find the newest published release including pre-releases (`/releases` list) instead of
  GitHub's `releases/latest`, which 404s while only pre-releases exist.
- Windows service host talks to the Service Control Manager.
- Windows auto-update task passes the server folder; git `safe.directory` is set.
- Service account home directory is `/nonexistent`; clearer free-space message.
- The FXServer folder is shown with the platform's path separators.

### Fixed
- Port check: a port held only on the loopback address (for example by an existing MariaDB) is now seen as in use on Windows.
- Windows: MariaDB install failed with "no checksum listed" (archive.mariadb.org prefixes names with `./`) and with a FATAL from `mariadb-install-db` (a `--config` file that did not exist).
- Windows: `sc start` no longer fails with error 1053.
- Windows: renames are retried after extract/clone/copy (antivirus holds new files); git long paths enabled.
- Windows: firewall detection on non-English systems; git/gh found right after winget installs them; PATH refreshed after winget; port-reserved hint.
- Windows: services can read the secrets file.
- Windows: winget "already installed" exit codes count as success; case-insensitive folder comparisons; MariaDB config quoting, ASCII-only folder, antivirus hints; home folder taken from USERPROFILE first; clearer Prepare/Review/failure wording.
- Paste in port fields keeps digits before the length cap is applied.
- `install.ps1`: no empty catch block (PSScriptAnalyzer).

## [0.1.0]

Initial pre-release.
