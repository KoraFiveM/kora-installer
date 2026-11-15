# Changelog

All notable changes to Kora Installer. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [SemVer](https://semver.org/).

## [Unreleased]

## [0.2.2] - 2026-11-15

### Added
- Manual update flow: `kora update <server>` shows what would change and asks before touching anything. Development servers fast-forward clean forks only; production servers move to pinned commits. A snapshot is taken first and an update that fails is rolled back automatically. Flags: `--resource <id>` (repeatable) or `--all`, `--preview`, `--yes`/`-y`, `--restart none|ensure|full|when-quiet` (default `when-quiet`), `--rollback <snapshot>`, `--list-snapshots`. `--auto` (what the scheduler runs) is unchanged and cannot be combined with the others. Newer Kora releases are detected as well.
- Discord notifications for update reports: `kora webhook set <server> [<url> | --stdin] [--test]`, `kora webhook test|show|remove <server>`. The webhook token is hidden in `show`; `--stdin` or `$KORA_DISCORD_WEBHOOK` keep it out of shell history. Reports are sent after automatic updates (`update --auto`).
- Configurable update schedule: `kora schedule show|set|off <server>`; `set` takes `--daily HH:MM` or `--weekly DAY HH:MM`, and `--as-user` (default) or `--as-system`. Stored in `<server>/.kora/schedule.json`. Development servers default to daily 04:00.
- The Kora app: the installer also installs `kora` on the machine so you can reopen it later. Windows: `%ProgramFiles%\Kora\kora.exe`, on the machine PATH, a "Kora" Start Menu shortcut (and optional Desktop shortcut) that asks for administrator permission, an "Apps & features" entry, and the Kora fox as the `kora.exe` icon. Linux: `/opt/kora/bin/kora`, a `/usr/local/bin/kora` command and a desktop launcher. `kora app install|status|uninstall` (`install` takes `--yes`, `--no-desktop-shortcut`, `--force`, `--dry-run`), `kora uninstall --app`, and `--no-app` / `--no-desktop-shortcut` on `kora install`. Removing the app never touches servers or databases.
- Real Linux end-to-end smoke tests in CI (basics, production install, service install, automatic update, uninstall), next to the Windows smoke tests, which now also cover the app install and run on pull requests.
- Dependabot for Cargo and GitHub Actions (weekly, grouped); supply-chain checks in CI (`deny.toml`).
- `kora self-update [--check] [--yes] [--version vX.Y.Z]`: updates Kora itself. Picks the newest release (pre-releases included), never downgrades unless you pass `--version`, verifies the SHA-256 before replacing anything and keeps a `.old` copy for rollback. A one-line "Kora vX is available" notice is shown in interactive terminals (checked at most once a day; off in CI and with `KORA_NO_UPDATE_CHECK=1`).
- `kora manager`: a screen for all your servers (list, details, start/stop, logs, update, webhook and schedule screens, add or adopt, delete). A bare `kora` opens it when servers exist and the install wizard otherwise.

### Changed
- Windows now installs MariaDB 11.4.13 from the official mirror network (about 300x faster than archive.mariadb.org at its slowest) and verifies a pinned SHA-256. The download shows progress and speed, resumes a broken connection, gives up on a source that is silent for 30 seconds and falls back to archive.mariadb.org. A file that does not match the pinned checksum is deleted and never installed.
- The automatic update runs as the person who installed (a "run only when logged on" task on Windows with no stored password; the installing user's git credentials on Linux) instead of SYSTEM, so private forks can be fetched. `--as-system` keeps the old behaviour. Schedules made by older versions are read back as SYSTEM. Linux timers add up to 10 minutes of random delay and catch up after downtime.
- README rewritten; added CONTRIBUTING, SECURITY, Code of Conduct, issue/PR templates and CODEOWNERS. CI cancels superseded runs.

### Security
Fixes from an internal security audit of 0.2.1 (no high-severity findings):
- Downloads follow at most 5 redirects and every hop must be HTTPS (previously a redirect could drop to plain http). A host such as `localhost.evil.example` no longer passes the local-testing exception. Requests time out after 1 hour.
- The Git/GitHub CLI installer is no longer downloaded to a predictable, user-writable temp folder and then run elevated; it now uses a private folder under `%ProgramData%\Kora\tmp` limited to Administrators and SYSTEM.
- On Windows, system tools (`icacls`, `sc.exe`, `netsh`, `powershell`, `git` and others) are now resolved from `System32` / PATH instead of the folder containing `kora.exe`, so a file planted in Downloads can no longer run with administrator rights. `sudo` is only taken from system folders.
- systemd unit paths are validated: server folders containing spaces or `% " \ $ ;` are refused (a folder such as `/srv/a /etc` could have made `/etc` writable to the service). Windows service paths reject `"`.
- The copy of `kora` that the service runs is re-hashed against the hash in its folder name before it is put in place; a swapped file is deleted and the install stops.
- `--license-key` is moved to the `CFX_LICENSE_KEY` environment variable for the elevated copy and removed from the relaunch command line, and Kora warns that a key on the command line is visible in process lists. Prefer the environment variable or the prompt.
- The MariaDB zip extractor now enforces its size limit while copying, not afterwards.
- Known and not fixed: the MariaDB root password is briefly in the arguments of `mariadb-install-db.exe` (administrators only); the MariaDB checksum list is not independent of the download host (now mitigated by the pinned 11.4.13 hash); GitHub Actions are pinned by tag, not commit; `KORA_*` environment overrides are honoured when elevated.

### Fixed
- The Windows MariaDB download no longer crawls or stalls: it shows progress, resumes, and falls back to a second source.
- The automatic update no longer fails on forks of private repositories because it ran as SYSTEM (see Changed).

## [0.2.1] - 2026-11-14

### Added
- The installer now starts your server when it finishes (final step "Start your server"): FXServer runs in the background (or through its service), Kora waits until txAdmin answers and tells you plainly if it does not come up (rejected licence key, port clash, crash, with the last log lines). The final screen has [ Open txAdmin ], [ Copy connect address ], [ Copy password ], [ Open folder ] and [ Stop server ] (or [ Start server ] when it is not running). `kora install --start` / `--no-start` for scripts (off by default there); `kora start`/`stop` work without a service; `kora status` shows the running state and port states.
- Development mode: the installer signs you in to GitHub (one-time code, copy and open buttons) and lets you pick where your forks go - your own account or one of your organisations (greyed out with the reason when one can't be used) - or create a new organisation. Your last choice is remembered.

### Fixed

### Changed
- The generated txAdmin password is no longer printed in the install notes or log; it is in `secrets/txadmin-admin.txt` and shown on the final screen.
- Kora now asks for administrator permission itself (Windows UAC prompt, or `sudo`) instead of telling you to "Run as administrator". On Windows, interactive commands (`kora`, `install`, `delete`, `uninstall`, `start`/`stop`/`restart`, `update`) reopen themselves elevated; `--version`, `--help`, `status`, dry runs and non-interactive use stay unelevated. `--no-elevate` turns this off.
- `kora status`/`start`/`stop` no longer treat a server without a systemd service as having a stopped one.

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
