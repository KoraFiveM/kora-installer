# Changelog

All notable changes to Kora Installer. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [SemVer](https://semver.org/).

## [Unreleased]

### Added
- `kora manager`: a full-screen manager for every registered server: start/stop/restart, open txAdmin or the folder, logs, ensure a resource, delete, add an existing server, and "Install a new server". Bare `kora` in a terminal opens it when at least one server is registered, and the install wizard otherwise.
- Manager > Update: a preview table (resource, from -> to, commits behind, needs restart, Clean/Dirty/Diverged/Up to date) with the changelog of the highlighted resource, tick resources or tick all, a restart choice (none / ensure / full / when quiet), live per-resource progress, the snapshot id, [ Roll back ] after a failed update and a list of saved snapshots to roll back to.
- Manager > Notifications: set, test and remove the Discord webhook; only the redacted address is ever shown. Manager > Schedule: see the schedule in plain words, change preset/time/day, choose "as you" or "System" (with the tradeoff explained), remove it; asks for administrator permission when needed.
- Update badges in the server list (`<server>/.kora/update-status.json`) and a [ Check for updates ] action that refreshes them.
- Manual update flow (`kora update <server> --preview | --resource <id> | --all | --rollback <id>`): preview what changes, a snapshot before every apply, rollback, production servers move to the catalog's pinned release commits. Discord notifications (`kora webhook set|test|show|remove`) for update reports. `kora schedule` (see 0.2.1).
- Every `kora update --auto` run, every full manual `--preview` and every full `--all` apply writes `<server>/.kora/update-status.json` (`{"checked_at": RFC3339 UTC, "updates": n waiting, "attention": n needing a person, "new_release": string|null}`, written atomically) so the manager can show badges.
- Applying now reports each resource as it happens (the engine streams events: snapshot saved, resource updating, updated, rolling back, checking the server) instead of only before/after.

### Changed
- `kora update --auto` exits **3** when something needs attention (dirty or diverged fork, SQL migration to run), like the manual flow; **1** still means failed or rolled back, 0 means fine. Update any script that treated every non-zero as "rolled back".
- A manual `apply` now sends the same Discord notification as the automatic update.

### Known limitation
- `--restart ensure` applies the files and does **not** restart or live-`ensure` anything: txAdmin 8.1.2 and FXServer expose no way to `ensure <resource>` on a running server without credentials (txAdmin's API and live console need a signed-in admin; rcon is off unless the owner sets `rcon_password`), and Kora does not store or reuse the txAdmin password. The apply output says so plainly: restart the server, or run `ensure <resource>` in txAdmin's live console, to load the new files.

### Changed
- MariaDB downloads now use the official mirror network (about 300x faster than archive.mariadb.org at its slowest) and are verified against a pinned SHA-256. Kora installs MariaDB 11.4.13 on Windows. The download shows progress and speed, resumes a broken connection, gives up on a source that sends nothing for 30 seconds and falls back to archive.mariadb.org; a file that does not match the pinned checksum is deleted and never installed.

## [0.2.1] - 2026-11-14

### Added
- The installer now starts your server when it finishes (final step "Start your server"): FXServer runs in the background (or through its service), Kora waits until txAdmin answers and tells you plainly if it does not come up (rejected licence key, port clash, crash, with the last log lines). The final screen has [ Open txAdmin ], [ Copy connect address ], [ Copy password ], [ Open folder ] and [ Stop server ] (or [ Start server ] when it is not running). `kora install --start` / `--no-start` for scripts (off by default there); `kora start`/`stop` work without a service; `kora status` shows the running state and port states.
- Development mode: the installer signs you in to GitHub (one-time code, copy and open buttons) and lets you pick where your forks go - your own account or one of your organisations (greyed out with the reason when one can't be used) - or create a new organisation. Your last choice is remembered.
- `kora schedule show|set|off <server> [--daily HH:MM | --weekly DAY HH:MM] [--as-user|--as-system]`: change when a development server updates itself, after install. The setting is stored in `<server>/.kora/schedule.json`.
- The Kora app: the installer now also installs `kora` on the machine so you can open it again later. Windows: `%ProgramFiles%\Kora\kora.exe` on the machine PATH, a "Kora" Start Menu shortcut (and an optional Desktop shortcut) that always asks for administrator permission, and an "Apps & features" entry. Linux: `/opt/kora/bin/kora` plus a `/usr/local/bin/kora` command and, in desktop sessions, a launcher. New `kora app install|status|uninstall`, `kora uninstall --app`, and `--no-app` / `--no-desktop-shortcut` on `kora install`. Removing the app never touches servers or databases.
- A bare `kora` opens the manager when servers exist and the install wizard otherwise.
- The Kora fox is now the `kora.exe` icon.

### Fixed
- The automatic update no longer runs as SYSTEM by default (SYSTEM has no GitHub sign-in, so forks of private repositories could not be fetched). It now runs as the person who installed: on Windows as a "run only when logged on" task (no password stored; a missed run happens at the next login), on Linux with the installing user's git credentials when they own the folder. SYSTEM stays available with `--as-system`. Schedules made by older versions are read back as SYSTEM. Timers now add up to 10 minutes of random delay and catch up after downtime.

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
