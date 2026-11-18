# Changelog

All notable changes to Kora Installer. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [SemVer](https://semver.org/).

## [Unreleased]

### Added
- Install wizard: a **Browse...** button next to the server folder opens the system folder window (in the desktop app, or, when the browser is on the same computer as Kora, a window Kora opens itself: PowerShell's folder dialog on Windows, `zenity` or `kdialog` on Linux, `osascript` on macOS). New local-only API call `pick_folder`.
- A troubleshooting page, "Players cannot join my server" (`/help/join?server=<name>`), with the steps in order: server running, try it here, try it on the home network, firewall, router port forwarding, providers that share one address (CGNAT), test from outside. Linked from the "Can players reach this server?" card; it shows the server's own game port.

### Fixed
- The panel's Uninstall ("Kora's shortcuts" item) now also removes the start-up entry, the Kora program and its PATH entry. The running panel is not stopped by it; it stays up until closed and does not come back.
- `kora uninstall` (all servers) now also cleans up the per-user install instead of only the shortcuts: it stops the background panel and removes its start-up entry, removes `kora.exe` (and `.previous`) and its user PATH entry, and removes the Windows Firewall rules for `kora.exe` (needs Administrator; otherwise the command to run is printed). Kora's own records (server list, database, sign-in token) are asked about and kept by default (`--delete-files` deletes them). Whatever is left is listed.

## [0.8.1] - 2026-11-18

A small Windows fix: the background panel no longer starts from a script file that antivirus products flagged.

### Fixed
- Windows: the background panel no longer starts from a VBScript file in the Startup folder, which antivirus products reported as a trojan. It now starts from a per-user `Run` registry entry that runs `kora.exe` itself (new hidden `--background` option, no console window). The old `Kora Panel.vbs` is removed the next time the service is installed, and by `kora panel --remove-service`.

## [0.8.0] - 2026-11-18

This is 0.8.0: one administrator service replaces the per-action permission prompts (with a Windows installer and a Debian package), a server's source is "clone or fork" and "released or latest" instead of Production and Development, every resource has its own page, and Kora gained backups, moving a server, container mode on Linux, HTTPS with passkeys, a SQLite data store, port checks and forwarding, an optional desktop app, and a first translation. Kora's own files are moved into a new data file on first start; read "Changed" before upgrading.

### Changed
- **Breaking: administrator rights come from one administrator service.** Windows `KoraService` and systemd `kora.service` do a fixed list of validated administrator operations for the panel, which keeps running as you. The Windows permission prompt for every install, the optional Linux helper, `kora helper`, `kora admin-worker`, the `X-Kora-Admin` header, the `admin.*` problem codes and the `request_admin` fix action are **removed** (they elevated a program in a user-writable folder and could not work with nobody at the desktop). `request_admin` became `install_admin_service`; the new problem codes are `service.not_allowed` and `service.lost`. Without the service the panel offers **Install Kora's administrator service**, **Continue without the administrator parts**, or **Show me the command**. `kora uninstall` and removing the old machine-wide app (`kora uninstall --app`) stay commands for an administrator terminal; the service never does them.
- **Breaking: "Production" and "Development" are gone.** A server has two plain settings: **where the resources come from** (plain clones, or your own copies on GitHub, in your account or an organisation) and **which versions** it follows (released versions or the latest changes). The wizard asks both, the servers list and Overview say "Plain copies, released versions" or "Own copies, latest changes", and the Resources tab can change the versions later. Existing servers keep working: `prod` reads as clones + released and `dev` as own copies + latest, both spellings are written, and `kora install --mode prod|dev` and the API's `mode` still work next to `--source clone|fork`, `--versions released|latest` and `--fork-owner`. See the table in docs/panel.md.
- **Branches, pull requests, the GitHub tab, the Schedule tab and the automatic update are for every server.** On a server that follows released versions, putting a resource on a branch or pull request is a deliberate, administrator-only choice with a clear warning; updates leave it alone; **Back to the released version** returns to exactly the pinned commit. A clone has only `origin`, so its pull requests are the original repository's.
- **Breaking: Kora's own data lives in one SQLite file**, `store/kora.db` in the Kora folder (bundled: nothing to install; see [docs/store.md](docs/store.md)): the server list, saved licence keys, remembered names, stopped installs and saved answers, remote-access settings and users, nodes, jobs and the activity log. The panel and a `kora` command running together can no longer overwrite each other's change. It is as private as `secrets.json` was (`0700`/`0600` on Linux including the `-wal`/`-shm` files, inherited permissions removed on Windows, links refused). Upgrading is automatic: on first start `servers.json`, `secrets.json`, `profile.json`, `last-install.json`, `panel.json`, `hub.json` and `paused-installs/` are imported in one transaction and checked, then **moved** (not deleted) to `store/before-sqlite/`; a file that cannot be read is never lost or replaced. A copy of the data file is made before every later change of its format. The way back to 0.7 is `kora store export-json`. Job errors stored by 0.7.0 still read.
- **Breaking: remote sign-in.** `kora panel user add` is now `kora panel user invite` (the old name still works): a new user gets a one-time link to set up a passkey or one-time codes instead of a token. `POST /users` answers `{name, role, link, expires_in}`. A user who has set up a passkey or codes can no longer sign in to the page with the old token (it still works as a script header). Passkey and code sessions last 8 hours idle / 3 days; token sessions at most 1 day (was 7).
- **Installers.** The MSI puts Kora in Program Files; the per-user install still works without administrator rights. The unattended-update task made through the service always runs as you, unelevated; "run as the system account" stays a command for an administrator terminal. `install.sh` and `install.ps1` offer the system install first (per-user stays available: `KORA_SYSTEM=0` / `-PerUser`).
- File sizes use decimal units from `Intl` (`152 kB`, `2.5 GB`) instead of hand-made 1024-based text; relative times likewise. Progress bars are green, like the main buttons.
- Errors that can reach a person are `Problem` values from where they are raised to where they are shown (`api::Res<T>` is `Result<T, Problem>`; `InstallError` and the packed-string form are gone). Low-level errors (files, JSON, internet, git) become one plain sentence with the technical text under "Show details". New codes: `io`, `data`, `network`, `storage_full`, `git.failed`, `server.not_kora`, `needs_db_password`, `tls.expiring`, `tls.missing`. A server folder without `kora-server.json` answers with `server.not_kora`. The JSON error shape is unchanged; a `Problem` may carry an optional `i18n: { id, params }`.
- Update schedules through the service take the server's folder, never a name looked up in the person's own list. The Linux service step hands the server folder to its own account through open folder handles (never `chown -R <path>`).
- Remote sessions: the remote-actions switch is worded "for sign-ins that do not come through Tailscale". `--allow-public` needs HTTPS or a trusted proxy.
- The panel loads faster: the first load is under 160 kB gzipped, and the wizard, Settings, Fleet, System check, sign-in screens and each server tab load when first opened. If Kora was updated while a page was open, the page says "This page needs to be reloaded" with a button instead of staying blank.

### Added
- **Kora's administrator service and its installers.** A **Windows installer (`.msi`)**: Program Files, the service, PATH, Start Menu and Desktop shortcuts, the optional desktop app (`DESKTOP=0` leaves it out), `ALLOWUSERS` for unattended installs, upgrade and uninstall in Apps & features. A **Linux `.deb`** (or `sudo kora service install` on other systemd distributions), and a separate `kora-desktop` `.deb`. ONE administrator approval at install time; after that the panel installs and removes servers' services, firewall rules and databases, changes update schedules and updates Kora with no prompt, from this computer or another device. Identity is read from the connection (Windows token SID and integrity, Linux `SO_PEERCRED`) on every request; every operation is checked on the privileged side (folder owner without following links, ports from the folder's record, the ownership ledger), runs with a clean environment, and is audited (no paths, no secrets). `kora service status|list|allow|deny|install|uninstall|migrate|finish-upgrade`; a standard account approved with an administrator's password at install is allowed too (`kora service allow-installer`). Updating Kora through the service keeps the previous program, checks the new one starts, and rolls back by itself after three starts that do not stay up. `GET /system` has `admin_service` (`ready` / `missing` / `not_allowed`); `POST /system/admin-service/install` is local only. See [docs/privileged-service.md](docs/privileged-service.md).
- **Migration from 0.7.0 and 0.6.x** (`kora service migrate`, run by the package and the MSI): the old Linux helper is removed and its account stays allowed; update tasks that still run with full administrator rights are rewritten to unelevated; servers set up the old way, including root-run installs, are recorded as yours; your earlier per-user copy of Kora is set aside (`.previous` added to its name). Servers and data are never touched.
- **A page for every resource** (`/servers/<server>/resources/<resource>`): where its files are, **Check for update**, **Update**, **Start**, **Stop**, **Restart** on the running server, each with progress and, when it cannot work, a plain reason and the fix. **"Restart needed: the running server still has the old version"** after an update, branch switch, pull request, rollback or kept-aside changes, kept per resource (`kora-resource-state.json` in the server folder) until the resource has really been restarted (also after a restart made outside Kora). A resource not on its normal version carries one badge everywhere: Resources list, its page, servers list, Overview, GitHub tab, snapshots, update check, `kora status`, `kora branch list`.
- **GitHub work on a resource's page:** open pull requests of the original and of your copy with checks and review state, details, comment, approve, ask for changes and merge (as the GitHub account the computer is signed in as, which every button says; never bypassing the repository's rules); try a pull request, **Get the latest commits**, **Stop trying it**; local changes line by line, commit, push (never forced), new branch, open a pull request; issues and releases; **Make my own copy (fork)**. Diffs never show `secrets.cfg`, `server.cfg`, key or password files and hide values that look like secrets; the activity log records pushes, comments, reviews, merges and forks with the account that acted. `kora resource show|changes|log|commit|push|new-branch|issues|issue-comment|releases|fork|versions` and `kora pr view|comment|review|merge` do the same.
- **Backups, restore and moving a server.** A **Backups** tab and `kora backup create|list|verify|restore|undo|delete|prune|schedule`. A backup is one archive (`.tar.gz`, or `.tar.gz.age` when password protected, opened by `age -d`) holding a consistent dump of the server's own database, its configuration, txAdmin's data, Kora's records and the resources (by reference where they can be fetched again, in full where they cannot). Made while the server runs or with **Stop the server during the backup**; owner-only from the first byte; only administrators can download it. **Restore** checks every file against its checksums, makes a safety copy, restores the database before any file, and **Undo this restore** puts the safety copy back. Backups record the server's source, versions and each resource's branch or pull request, and restore them exactly. **Move this server** makes a password-protected file; **Bring a server from a backup or another machine** (install wizard, `kora backup import <file>`, upload up to 20 GB) shows what will be created and what does not come along, creates a **new** database and login, and carries on from where it stopped after a failure. Cfx.re ties a licence key to your account and the server's address: Kora moves the key but cannot make Cfx.re accept it elsewhere, and says so. Retention (newest N and/or one per day for D days), a free-space check first, and "No backup yet" / "Last backup 3 days ago" with a warning on the Overview. Deleting a server keeps its backups, listed under "Backups of servers that no longer exist". Operators may make and check a backup; restoring, deleting and downloading are administrators', and need `kora panel remote-actions on` from a token sign-in.
- **Scheduled backups** (daily or weekly, off by default) run as a systemd timer or Task Scheduler task named `kora-backup-<server>` when the administrator service is installed for your account (running `kora backup create <folder> --scheduled` as you, whether or not the panel is open), and from the panel's background service otherwise; `kora backup schedule` says which. A missed slot is made up once. A failed one raises an alert and goes to the server's Discord and webhook.
- **Restores and imports hand the files to the server's own account** through the service (`kora hand-over`; the account comes from the server's own record), and say plainly, with the command, when that could not be done.
- **Container mode (Linux).** The install can run a server in a rootless **Podman** (preferred) or **Docker** container instead of as a system service (`kora install --run-mode container --engine podman|docker`; offered only when an engine really ran a test container). One container per server on a pinned Alpine image (Kora publishes no image), the game port published for TCP and UDP, txAdmin's on loopback only, `--cap-drop ALL`, `no-new-privileges`, never `--privileged`. The database is by default a MariaDB container next to the server; "use an existing database" stays. Start, stop, status, logs, the console, txAdmin, updates, resources, backups, the monitor, the reachability check and delete all work. The System check has a **Containers** line with guided fixes. Not in this release: converting a service server to a container (or back) in place.
- **HTTPS for the panel.** Any listener other than loopback serves HTTPS once there is a certificate: your own (`kora panel tls set --cert --key`), Tailscale's (fetched and renewed by itself) or a self-signed one from a local CA you trust once. Plain HTTP from other computers is redirected, cookies get `Secure`, expiry warnings. Settings > HTTPS and `kora panel tls status|set|self-signed|tailscale|auto|off|renew|export-ca`. **A reverse proxy you trust, on purpose** (`kora panel proxy trust|forget|status`), also for txAdmin (`kora panel proxy txadmin-host`), with Caddy and nginx examples in docs/panel.md.
- **Passkeys, one-time codes and API tokens for remote users.** People invited with `kora panel user invite` (or Settings) set up a passkey (WebAuthn) or a password with one-time codes and recovery codes. Lost device: `kora panel user reset`. "Your sign-in" in Settings lists passkeys, codes and signed-in devices. Destructive actions from such a session ask for a fresh passkey or code check. API tokens (`kora panel user api-token ...`) replace tokens for scripts: header only, never in an address. `kora panel remote require-strong on|off|status` requires a passkey or codes for everybody. Sign-ins, failures and resets go to the activity log.
- **Saved licence keys in the system keychain**: Linux Secret Service and Windows DPAPI, with the private file as the fallback. Settings and `kora keys list` say where each key is kept; `kora keys keychain status|on|off` moves them in and out.
- **Jobs survive a restart and an activity log.** Updates, installs and the like are recorded as they run (secrets never written); a job running when the panel closed shows as "stopped by a restart" with what to do next. An append-only **Activity log** (Settings, `kora audit`; kept 365 days) records every change made from the panel or `kora` (when, operation, server, who, result; never a secret), including what the administrator service did (`kora audit --service`, "Done with administrator rights"). `kora store status|backup|restore|export-json`.
- **"Can players reach my server?"** A checklist on each Overview, the install's last page and `kora reach <server>`: running (TCP and UDP, not only on this computer), firewall, router, internet, each OK / probably OK / problem / not known with exact values to enter where Kora cannot act, and a plain explanation of carrier-grade NAT. Nothing is contacted without your yes: Cfx.re's public server list and a "what is my address" website each have a switch, off by default. **Automatic port forwarding** (UPnP, PCP, NAT-PMP; off by default, per server) forwards only the game port, only to this computer, is kept while the server runs and removed when it stops or is deleted. Better connect addresses on the Overview. "Open the firewall for this server's game port" goes through the service.
- **txAdmin through the panel.** "Open txAdmin" from another computer used to guess a port that was usually closed. The panel now opens one more port for that server's txAdmin (txAdmin's port + 10000) on the address the panel is reached at, signs you in with your panel session and passes pages, cookies and the live console through. Administrators only, and a risky action (a token sign-in needs `kora panel remote-actions on`).
- **The Kora desktop app (optional)**: a tray icon (green, amber, red or grey) and a native window showing the same panel (WebView2 on Windows, WebKitGTK on Linux), with start, stop and restart for each server, notifications that use your alert rules, start with Windows or your desktop, and system dialogs for choosing files. Part of the MSI (optional feature) and a separate `kora-desktop` Linux package. The Kora shortcut opens it when installed. A thin client of the panel: nothing changes for the browser, remote use or a server without a screen. See [docs/desktop-app.md](docs/desktop-app.md).
- **Support bundle** (Settings, a link in every error's details, `kora support bundle [<server>]`): one zip with versions, the system, the System check, tool versions, servers, `server.cfg` and Kora's records, recent jobs and problems, the activity log and the service, firewall and disk state. Kora shows what is in it before anything is saved, saves it owner-only and **never uploads it**. One deny-by-default redaction removes licence keys, webhook addresses, tokens, private keys, password hashes, secret-named settings and user names in paths; files known to carry secrets are listed, never read. The same function cleans resource diffs, the job history and the text of issue reports.
- **Translations.** The panel can be shown in other languages: a language picker in Settings and on the sign-in screens (remembered per browser, defaulting to the browser's language), every panel text in a message catalogue, dates, numbers and sizes through `Intl`. **German (beta)** ships first, machine-assisted and awaiting a native review. See [docs/translations.md](docs/translations.md). Server messages can carry an id so the panel shows them in your language.
- **Accessibility pass (WCAG 2.2 AA)** ([docs/accessibility.md](docs/accessibility.md)): one `h1` and a sensible outline per page, a skip link, focus and page titles that follow route changes, progress announced at its start, each quarter and its end, field borders and placeholders with enough contrast, no sideways scrolling at 320 px, reduced motion, table captions, and dialogs that return focus to their button. The browser tests' axe gate fails on moderate findings and every page, tab and dialog is swept.
- **Alerts that belong to no server** (HTTPS certificate ending or expired, the administrator service not answering, a scheduled backup that cannot run) use the alert list, the desktop notification and the Discord and webhook addresses of servers that allow them.
- Errors that say what to do carry on by themselves; **Bringing a server in asks for a database administrator login** when the automatic one does not work (used once, never stored).
- Windows code signing is ready to switch on (`docs/release-signing.md`); releases stay unsigned until it is configured.
- Developer-facing: the panel's TypeScript types and client are generated from the Rust API description; Playwright browser tests (`web/e2e/`); a Windows administrator-path CI job.

### Fixed
- **The Windows installer no longer hangs**: `kora service allow` (run by the MSI) waited for "Press Enter to close this window" in a console nobody can see. Commands that installers run never wait for Enter.
- Windows: an account allowed after the service started was refused for ever; `kora service allow|deny` now takes effect at once. A client that connected and sent nothing held a service thread for ever (sixteen stopped the service answering); it is now cut off after ten seconds.
- Windows installer: upgrading from an installer without the desktop app keeps a Start Menu shortcut ("Kora Panel" starts the panel; "Kora" the desktop app); an upgrade no longer takes out the desktop app's start-up entry; `kora.exe.old` is removed at uninstall; the service restarts after an error exit.
- Windows: files only their owner may open (the panel's sign-in record, saved settings) are locked to the account Windows says is running, not the `USERNAME` variable, which could lock a user out of their own files.
- Programs run by installers and the service (`sc`, `schtasks`, `netsh`, version checks) get no input and a time limit, and a grandchild that keeps their output open no longer keeps Kora waiting.
- A guided error (with its fix buttons) was flattened into plain text when Kora added words in front of it: starting or stopping a server, reading the server list, remembered answers and paused installs (a busy file lost its "Try again"), the choices on installing Git, and the catalogue and automatic-update jobs. An update refused because another is running is recognised by its code, not its wording.
- The Resources tab of a server Kora did not install asked for a catalogue that does not exist (the panel answered 422 and the browser logged an error); it now says resources can only be added to servers Kora installed.
- Resource pickers show the one-line description of each resource again. A resumed install keeps its txAdmin admin. The `secrets/txadmin-admin.txt` note points to the Open txAdmin button, not an SSH tunnel.
- Accessibility: red text and solid red buttons had too little contrast, a link in the wizard was told apart only by colour, several buttons had an accessible name that did not contain their visible text, and dialogs lost the keyboard position when closed.

### Removed
- `kora helper`, `kora admin-worker`, the Windows permission prompt, the Linux helper, the `X-Kora-Admin` header, the `admin.*` problem codes and `request_admin` (see Changed).
- The wording "Production" and "Development" in the panel and the help (the old `--mode` spelling and the stored `mode` still work).
- The older, thinner redaction copies (one function now), and the packed-string form of a `Problem`.

### Security
- Nothing asks for administrator rights per action any more: the privileged side is one small service in a folder only administrators can change, with a typed allow-list, strict identity checks on every request, an ownership ledger and its own audit; a child of the service never opens a person's data file.
- HTTPS for remote listeners, passkeys or one-time codes with re-authentication before destructive actions, API tokens that are header-only, token sessions shortened to one day, and a lockout that uses the visitor's address behind a trusted proxy.
- Backups and support bundles are owner-only from the first byte, backups can be password protected (`age`), and one deny-by-default redaction function cleans everything Kora shows or saves for support. Saved licence keys can live in the system keychain.
- A diff of a resource never shows secret files or values that look like secrets; txAdmin through the panel is administrator-only.
- New dependencies: `age`, `regex`, `rustls`, `tokio-rustls`, `rcgen`, `x509-parser`, `webauthn-rs` (MPL-2.0, allowed in `deny.toml` for those crates only), `argon2`, `hmac`, `sha1`, `qrcode`, `secret-service` (Linux) and `windows-sys` (Windows).

## [0.7.0] - 2026-11-17

This is 0.7.0, "panel first": the web panel is the only interface, nothing needed to get started asks for administrator rights, and failures say what to do and offer the button to do it.

### Fixed
- **Fixes carry on by themselves.** When "Install it for me", the GitHub sign-in or "Start it for me" works from a failed install step, the wizard retries that step automatically and says so (it used to say "you can try that again now"). The same goes for a failed start of the install, the wizard's database test and anything that has its own "Try again". Fix buttons are only drawn for administrators; others are told who can fix it.
- A stopped install is one banner in the wizard ("Carry on" or "Change my answers first"), and counts once for its saved licence key. The disk-space message no longer talks about `--dir` and environment variables, and a full disk points at the folder first. The finished-install page gives the same txAdmin address, connect note and folder button as the server's Overview. One name ("licence key", "pull request") throughout.

### Changed
- **The web panel is the only interface.** Running `kora` (typed, double-clicked or from a shortcut) makes sure the background panel runs and opens your browser at it, and never asks for administrator rights. With no servers yet the panel opens straight into the install wizard (once per browser tab; "Servers" still shows the empty list with "Add an existing server", and the install page links to it: `/?add=1`). On a computer with no screen (SSH) `kora` prints the sign-in link, a ready-to-paste `ssh -L` command and the Tailscale option instead. If the panel is already running it is just opened; if the port is taken the next free one is used; if no background service is possible it runs in that window. Started with `sudo` (Linux) it runs again as the person who typed `sudo`; from an administrator window or a root shell it opens a running panel (on Windows, when the background panel was set up before, it starts it as your normal user through Explorer) or explains what to do. `kora panel --foreground` run as root says the same.
- **`kora manager` and `kora install` with no options are now the same as `kora`** (`kora install` opens the install page). `kora install` with options is unchanged: the scriptable installer.
- **Per-user shortcuts replace the machine-wide "Kora app".** The install scripts and `kora app install` make shortcuts that open the panel and belong to you alone: Start Menu and Desktop on Windows (no administrator rights, not "Run as administrator"), a launcher in the applications menu on Linux desktops (no terminal window). `KORA_NO_SHORTCUTS=1` / `-NoShortcuts` skips them. The server install no longer adds the app in `%ProgramFiles%` / `/opt/kora`, and its options are gone from the wizard (`--no-app` and `--no-desktop-shortcut` are still accepted and ignored).
- **Breaking: `kora app uninstall` is per-user.** It removes Kora's own shortcuts (Start Menu and Desktop on Windows, the applications-menu launcher and its icon on Linux), with `--dry-run`, and needs no administrator rights for that. The OLD machine-wide app from earlier versions is removed with `kora uninstall --app` (as administrator or with `sudo`); `kora app uninstall` run as administrator removes it too, otherwise it says so. `kora uninstall` removes the per-user shortcuts as well (not under `sudo`: they are the user's, so it says to run `kora app uninstall` without it) and the panel's Uninstall list names them.
- **Breaking: the default folder of `kora install` on Windows is `C:\Kora\<name>`**, the same as in the panel (it used to be `%USERPROFILE%\kora\<name>`). Servers already installed keep their folder.
- **Breaking: API errors carry more.** An error may now have `details` (technical text) and `actions` (what the person can do), and its `code` can be a dotted one (`github.gh_missing`, `install.step_failed`, `keys.not_found`, `remote.actions_off`, ...) where it used to be a general one; failures a person can act on answer 422 (403 for `needs_elevation`, which still carries `command`). Scripts that match on error codes need checking: the list is in `crates/kora-core/src/problem.rs`.
- **The install no longer asks you to close the window or run the installer again** when git was just installed, the GitHub sign-in did not finish or permissions are missing: it says what happened and what to do. `kora install` in a terminal prints the same plain message.
- **Remote installs.** Administrators signed in over Tailscale can now install servers and do the destructive actions with no switch to flip first (their identity is verified by the Tailscale service); the operations that stay on this computer (remote-access settings, users, other panels) are unchanged. Administrators signed in with a user token can now install servers and manage them by default; deleting or unregistering a server and uninstalling Kora (and console commands, updating Kora and changing schedules) stay off for tokens until `kora panel remote-actions on` is run on the machine, and the refusal explains why, shows that command and names the other ways. `InstallStart` and `InstallResume` moved out of the destructive class; viewer and operator roles are unchanged. `kora panel remote-actions` now only affects token sessions.
- **A resource on another branch or a pull request is left alone by updates**, manual and scheduled. The update check and the report say "on branch fix-hud" or "trying pull request #12; not updated" (information, not a problem), nothing is pulled into your branch and it is never reported "up to date" by mistake. Going back to the normal version makes updates work for it again. Each snapshot now records the branch a resource was on, and a rollback goes back to that branch before it puts the old commit back, so it can never reset a branch you switched to since (older snapshots have no branch and behave as before). Production servers are not affected: they stay on their pinned releases.
- `kora uninstall` (all servers) keeps your saved keys and remembered names unless you answer yes to a question about them (default: keep; `--yes` keeps them), and asks before removing Kora's own Git and GitHub CLI copies.

### Added
- **Errors that say what to do, with buttons to do it.** When an install, a GitHub action or something that needs administrator rights fails, the panel says in plain words what happened, keeps the technical part under "Show details", and offers buttons for the fixes (try the step again, change an answer, sign in to GitHub, install the missing tool, the command to run as administrator, ...). Other errors get "Try again" and "Copy details".
- **A failed install step can be tried again, or skipped when that is safe, without starting over.** The install stops at the failed step and waits: "Try this step again" carries on from there with the same answers, "Change my answers" goes back to the summary. Only optional steps can be skipped: opening the firewall, the daily update schedule, and starting the server. A pasted licence key that was not saved is kept in Kora's memory only, never written to disk.
- **Saved licence keys, picked by name.** Keys are kept in one private file in the Kora folder, `secrets.json` (created readable only by you: mode 0600 on Linux, inherited permissions removed on Windows, like the per-server `secrets.cfg`; plain text, not encrypted). The install wizard offers a picker ("Main", "Test server", ...) and "Add a new key"; a pasted key is saved by default (name suggested) with a clear switch to not save it. Settings has a **Licence keys** card to add, rename, replace and delete keys, showing which servers use each. The panel and the API never return a full key, only its name and last four characters, and keys never appear in logs, job events, saved answers or error details. Only admins (and this computer) can add, rename, replace or delete keys; other roles that may install or change a key can only pick from the saved names. `kora keys list|add|replace|rename|remove|use` does the same from a terminal (a key is read from `CFX_LICENSE_KEY` or standard input, never from the command line), `kora install --license-key-name <name>` installs with a saved key and `--save-license-key <name>` saves the one in `CFX_LICENSE_KEY`.
- **Change licence key** on a server's Overview tab (and `kora keys use <server> <name>`): pick a saved key or add a new one; Kora writes it where the server keeps it (`secrets.cfg`) and says plainly whether a restart is needed (offering it when the server is running).
- **Stopped installs survive a panel restart when they use a saved key.** The stopped install is written down without any secret (the key is looked up by name when it carries on) and the install page offers "Carry on". A pasted key that was not saved is gone with the restart, as before. Renaming a key updates the unfinished installs and saved answers that use it; deleting one says what will need another key.
- **Remembered Cfx.re name and GitHub owner.** After first use they are kept in a separate non-secret file (`profile.json` in the Kora folder) and pre-filled on the install page, each editable; Settings shows them and can forget them (`kora profile show|forget`).
- **Kora keeps its own Git (Windows) and GitHub CLI (Windows and Linux).** Development installs no longer need `winget`, an administrator prompt or a "close this window and open a new one": Kora downloads a pinned, checksum-verified copy from the official GitHub releases (MinGit and `gh`) into its own folder (`%LOCALAPPDATA%\Kora\tools`, `~/.kora/tools`), unpacks it safely, moves it into place in one step (an interrupted download leaves nothing broken) and uses it without touching the `PATH`. A Git or GitHub CLI that is already on the computer is used first. Git on Linux stays the system package. `kora tools status` shows which copy is used; `kora tools install` sets them up.
- **System check page** in the panel (and `kora check`): Git, the GitHub CLI, the GitHub sign-in, the database and administrator rights, each with its state in plain words and a fix button where something is wrong: **Install it for me** (with a progress bar and, when the download fails, Try again and a manual link), **Sign in to GitHub** (also when only some permissions are missing), **Start it for me** for a database that is installed but not running (`kora check --start-database`: Linux uses the helper when it is installed, otherwise shows the command; Windows asks for administrator rights through the usual `needs_elevation` answer). The same buttons appear wherever an error carries them (the install wizard, the GitHub tab), and those errors link to the System check. The GitHub status tells "the GitHub CLI is not installed", "not signed in" and "signed in without the permissions Kora needs" apart.
- **Change a resource's branch, or try a pull request, from the panel** (Development servers only, administrators only). The GitHub tab has "Branches and pull requests": see which branch each resource is on, switch one to another branch (from your fork or the original repository), try a pull request by picking it from a list or pasting its number or address, and go back to the normal version with one click. The list of pull requests comes from both the original repository and your fork, each labelled with its title, number, author and where it is from. "Several resources at once" shows which resources have a branch before it switches any of them, and can put every resource back to the normal version. The Resources tab shows each resource's branch next to its name. The same from the terminal: `kora branch list|show|switch|default|all|restore` and `kora pr list|checkout`. After a switch Kora says the running server still has the old version, and offers to restart the resource.
- **Your own changes are never lost by a switch.** If a resource has changes you have not saved, Kora does nothing until you choose: keep them aside (they are listed afterwards, with "Bring my changes back"), throw them away (asked twice, with a warning), or cancel. Commits that belong to no branch are saved on a branch of their own before Kora leaves them. Kora never deletes a branch or a kept-aside set of changes, never pushes, and never forces anything.
- **Quick buttons on every server** (the servers list and the server's Overview): **Open txAdmin** (for someone reaching the panel from another computer it uses the name they used, and says what to do when txAdmin's port is closed or they are on an SSH tunnel), **Open folder** (opens the file manager when you are at the computer that runs Kora, and shows the install path next to it) which becomes **Copy path** when the panel is reached remotely or the computer has no desktop (an SSH tunnel counts), and **Copy connect address** (this computer's network address with the server's game port; the page says plainly that players outside your network need your public address and a forwarded port).
- **Tidy up old shortcuts.** When an earlier Kora left its machine-wide app and administrator shortcuts behind, Settings (and a notice on the servers screen) offers one click to make the new shortcuts first and then remove the old copy. Removing it needs administrator rights, so the panel shows the command to run (`kora uninstall --app --yes`) or uses the helper. `kora app status` shows what is found.

- **Windows: the panel asks Windows for administrator permission itself.** An install from the panel no longer stops at "run this command as administrator": Windows asks once when the install starts (the wizard says so beforehand), and the background service, firewall rule, update task and Kora's own MariaDB are set up while you watch the same install. The update schedule, Kora's self-update, "Tidy up old shortcuts", uninstalling, deleting a server that has a service and the System check's "Start it for me" (database) ask once each. If Windows does not get permission (No, nobody answered, or you are signed in from another device, where Kora only asks when you say so) you can ask again, continue without the administrator parts, or see the command. The install's progress and files stay yours, so a failed administrator step can be retried or, when safe, skipped. Behind the prompt is a short-lived elevated copy of Kora that runs only Kora's fixed list of administrator steps (see SECURITY.md).

### Security
- Pre-release review of the 0.7.0 changes. When Kora runs as root through the helper, inside your own Kora folder, it no longer writes `profile.json`, the stopped-install files or `last-install.json` through a link left there (they use a fresh temporary file renamed into place), and files it hands back to you (`secrets.json` and the others) are changed with `lchown`, so a link swapped in meanwhile cannot make root give another file to your account.
- `git` and `gh` are started by the full path of the copy Kora found, never by name, so a `PATH` with an empty or relative entry can no longer make the operating system pick a program from the resource's own folder (a pull request's files) while Kora runs `git` there.
- On Windows a job address such as `CON` or `NUL` can no longer be used to read a device instead of a stopped-install file; a literal `%` in the program path of the Linux launcher is written `%%`.
- **Windows administrator worker hardened** (review of the elevated worker). Its elevated children get an environment built from nothing (system folders from Windows itself, a system-only `PATH`, your own profile folders even when another administrator typed the password), so a variable you or a program set (`KORA_MARIADB_HOME`, `KORA_SELF_UPDATE_API`, `ProgramFiles`, `TEMP`, ...) can no longer steer them; the worker forgets those variables too. Kora's program file is locked from the prompt until the work is done, and the worker checks the panel by file identity, not by path text (8.3 names and `subst` drives work). An elevated Kora never takes a database client from `PATH`. One administrator session at a time, its steps one by one; a step is stopped when the panel goes away or after 45 minutes. Elevated steps, schedule changes and deletes re-check that the server folder is yours (no links on the way), take servers by name from your list (never a folder path), and keep a record of which Windows account each service, firewall rule, update task and database was made for, so one account cannot change another's. The elevated uninstall no longer deletes folders: the panel deletes them afterwards, as you. The firewall step opens no port below 1024 and no remote-administration or database port. A retried service step uses the service an interrupted attempt made instead of failing.
- **Windows: the daily update task runs without elevation.** It used to run with your full administrator token ("Run with highest privileges") on a folder other programs can change; now it runs as you, and you may start and stop that server's service (nothing more), which is all the update needs. Server services keep only the privileges an ordinary user has (no "impersonate a client"). An elevated Kora on Windows now hardens `git` as root does on Linux (no hooks, fsmonitor or repository-chosen programs). A task made by an earlier version keeps its old setting until the schedule is changed once.
- The pull request dialog says that a pull request is someone else's code that then runs on your server. `SECURITY.md` is corrected where it overstated where Git and the GitHub CLI are looked for, and says what a pull request, a token and a saved key can and cannot do.

### Removed
- **The terminal screens** (the `kora-tui` crate: install wizard and server manager). Everything they did is in the panel or the command line. `kora manager` remains only as another name for `kora`.
- **The Windows `winget` / vendor-installer route for Git and the GitHub CLI** (it needed administrator rights and a restart of every open window); the private copies replace it. On Linux the GitHub CLI is no longer installed from GitHub's apt repository either.
- The machine-wide app from the server install and the wizard (see Changed). Code nothing used any more (the old app's install planner now only exists in the tests that set up an old install), the `BSL-1.0` licence from `deny.toml` (only the removed clipboard crate needed it; `ryu` is `Apache-2.0 OR BSL-1.0`), and the last wording that mentioned the terminal screens.

## [0.6.5] - 2026-11-17

### Added
- **Progress bars for checking and updating.** Check for updates shows "N of M resources checked" and the resource it is on, and a single-resource check shows a small bar on its own row. The update progress card shows "N of M updated", counts failed resources as done, and names the step in progress. The check bar stays at "Starting" until the GitHub fetch returns, then moves through the resources.

## [0.6.4] - 2026-11-17

### Added
- **Coloured console and logs, like the game and txAdmin.** The server's colour codes now reach the panel: the live console and the Logs tab show the colours the server prints (bold and the 16, 256 and true colour codes), on the dark theme. Everything else (cursor movement, titles, control characters) is still removed, and the text is never read as HTML.

### Changed
- The panel's name in the sidebar and on the sign-in page is the Kora wordmark from the website, not text.
- The Updates tab is gone: its check and update flow sit at the top of the Resources tab, and clicking a resource's name checks just that resource (with an update button when it has a simple change).

## [0.6.3] - 2026-11-16

### Changed
- **A much shorter install wizard.** A default install is three screens: your licence key, the server's name, and a summary of everything Kora will do, each line with a "Change" link. "Choose everything myself" brings back the step-by-step wizard. Ports are checked before you click Install.
- Plainer words in the panel: "Sign in to Kora" with where to get a link, "Start a resource with the server" instead of "Ensure a resource", clearer help when a resource has changes of your own, a Start button where the console says the server is stopped, and next steps on common errors.
- When the server stops while starting, the message now says what to do next: a port clash names the port (when the server's log shows it) and points to the panel's Ports tab; a crash says the full log is in the panel's Logs tab.
- `kora --help` no longer lists the commands that only the operating system runs (`service-host`, `helper serve`) or the removed `--code` option. They work as before.
- The live console empties its command file (`.kora/console.in`) when the server starts, so it no longer grows forever. Servers get this the next time the live console is switched on for them (new installs have it).

### Fixed
- **Start, Stop and Restart of one resource used the wrong name.** The panel sent the catalog id (`kora-lib`) where FXServer expects the folder name (`kora_lib`), so the command did nothing for most resources. It now sends the folder name.
- "Start a resource with the server" (ensure) wrote the catalog id into `server.cfg` for the same reason; it now writes the folder name.
- Turning on the live console while the server runs no longer accepts commands that would never run: until the server has been restarted, the console and the resource buttons answer "The live console starts working after the server's next restart. Restart the server, then try again."

## [0.6.2] - 2026-11-16

### Added
- `kora panel restart` restarts the background panel and prints the sign-in link. A panel running in a terminal window is not touched: it says to stop it there and run `kora panel` again.
- `kora panel remote-actions on|off|status` lets remote administrators install, delete and uninstall servers, send console commands, change schedules and update Kora (off by default). Until now this switch was only in Settings on the computer itself, out of reach on a server with no screen. A running panel uses the new setting right away.
- **Start, stop and restart one resource from the panel.** Each installed resource on a server's Resources tab has Start, Stop and Restart buttons (Stop and Restart ask first). They need the server running and the live console switched on, and work for operators, also over remote access. `POST /servers/{s}/resources/{name}/control`.
- The installed-resources list has a filter and shows what the last update check found beside each name.

### Changed
- **The Resources tab remembers what it was doing.** A running check, what it found, what you ticked, an update or an install in progress all stay when you open another tab and come back (until the page is reloaded). The Updates card and the installed list share one state, so they no longer disagree or run into "Another update is running for this server".
- The Updates card lists only resources that need a decision and counts the rest; "Everything is up to date" is said only after every resource was checked.
- **Buttons you may not use are switched off with the reason**, instead of failing after the click: installing a server, schedules, ports, webhooks, GitHub, alert settings, rollback and adding resources follow your role and the remote-access switch.
- The install wizard sets up the database for you by default (uses MariaDB or MySQL if present, installs MariaDB if not), and the licence key step links to the Cfx.re portal with numbered steps.
- Removing a webhook, a remote user or a Tailscale account asks first.
- When an action is turned off for remote sign-ins, the message now gives the command that turns it on (`kora panel remote-actions on`).
- On Linux, `install.sh` and `kora panel` run as root now print the commands to create a user for Kora and continue, for servers that only have the root account.
- The install commands in the README use `releases/latest/download`, so they no longer name an old version.

### Fixed
- **Server status in the panel updates by itself again.** The panel listened for the wrong live event, so a server that stopped or started kept its old status until the page was reloaded.
- One lost request no longer turns a running job into "failed" in the panel; it asks again a few times first.
- A double click can no longer start two update checks.
- Starting or restarting a resource, and "ensure" with a restart, are refused while an update is swapping files.
- The "needs attention" alert pointed to an Updates tab that no longer exists; it now opens the server's Resources tab.

## [0.6.1] - 2026-11-16

### Security
- **Kora Panel no longer starts as root/administrator.** `kora panel` refuses to run elevated (the install script also skips starting it from a root shell and prints the command to run as your normal user), matching what SECURITY.md already promised. `KORA_ALLOW_ROOT_PANEL=1` overrides it for containers where root is the only account.
- **`git` run as root (the helper, `sudo kora ...`) ignores more of a repository's own settings.** Besides hooks and fsmonitor, `core.sshCommand`, `core.gitProxy`, `core.askPass`, `core.pager`, `core.editor`, `diff.external`, `credential.helper`, `protocol.ext.allow`, `uploadpack.packObjectsHook` and the matching `GIT_*` variables are pinned, so someone who owns a server folder cannot make root run their program through `.git/config`.
- `install.sh` writes the new program through an exclusively created temporary file (`mktemp`) instead of a guessable name.

## [0.6.0] - 2026-11-16

### Added
- **Restart now after an update.** When the background panel updates Kora, Settings offers "Restart now": the panel restarts on the new version and the page reloads (sign in again afterwards). A panel running in a terminal window says to stop it and run `kora panel` again.
- **Open Kora Panel from your own devices with Tailscale, in one step.** Run `kora panel --tailscale` on the server: Kora makes the Tailscale account you are connected from (or the account that added the computer to Tailscale, or `--admin you@example.com`) the panel's admin, keeps the background panel listening on this computer and on its Tailscale addresses (never the public one), lets the port through `ufw` on the Tailscale interface only (as root; otherwise it prints the command), and prints the address to open, for example `http://myserver.tail1234.ts.net:47120`. Open it on any device signed in to Tailscale and you are signed in with your Tailscale account: no link, token or SSH tunnel. The install script offers it on a server with no screen when Tailscale is running (`KORA_TAILSCALE=1` says yes in advance, `KORA_TAILSCALE=0` never asks).
- `kora panel tailscale allow|remove|list|off` lets other Tailscale accounts in (each with a role), removes them, or turns it off, without opening the panel.
- `kora panel --new-token` makes a new sign-in token for the background panel.
- **Release files can be checked against the build that made them.** Every release file has a signed build provenance attestation from GitHub Actions: `gh attestation verify kora-linux-x86_64 --repo KoraFiveM/kora-installer`. See [CODE_SIGNING.md](CODE_SIGNING.md).
- `KORA_PRERELEASE=1` (Windows: `-Prerelease`) lets the install scripts take a pre-release.

### Changed
- **Sign-in links work once.** The link printed by `kora panel` holds a one-time code (valid 10 minutes) instead of the long-lived token, so the token no longer ends up in your browser history. Run `kora panel` or `kora panel --print-link` for a new one. The background panel no longer writes a sign-in link to the system log.
- Opened from another computer, the panel signs you in with your Tailscale account straight away when that account is allowed. Settings > Remote access shows the Tailscale address.
- **The install scripts install the newest stable release**; pre-releases only when asked (`KORA_VERSION`, `KORA_PRERELEASE=1`). **`kora self-update` stays on stable releases** unless you run a pre-release or pass `--version`.
- **Downloads stay on https**: the install scripts refuse a redirect to plain http, and `install.sh` runs nothing if a `curl | sh` download is cut short.
- Every GitHub Action in the build and release workflows is pinned to a full commit SHA, and CI fails if one is not.

### Fixed
- Paste a sign-in code or a whole sign-in link into the sign-in box; signing in again ends the old session. A failed sign-in in a new tab no longer signs out your other tabs.
- Installing into `/home/kora` through the helper works when `/home` is a symbolic link; `kora panel --install-service` works when `~/.config/systemd/user` is one.
- `kora self-update` finds the latest stable release behind many pre-releases. `install.sh` works with BusyBox wget (https only).
- `kora panel --bind <address> --remote` no longer prints the 127.0.0.1 link of the panel already running in the background and stops: it starts the panel on that address.

### Security
- **Root no longer trusts files in a server folder.** That folder belongs to the server's own account (which runs third-party resource code) or to you, so before acting as root or administrator Kora now checks everything it reads there:
  - The automatic-update job (root / SYSTEM) only runs a kora program nobody else can change (Kora's root-owned copy in `/opt/kora`, Windows `%ProgramFiles%\Kora\bin`, or `kora` itself when installed in such a place), never a program named in the server's records. Before, anyone who could write the server folder could make root run a program of their choice. The Windows task definition is written to `%ProgramFiles%\Kora\tasks`.
  - `kora delete` and `kora uninstall` only delete inside the server folder and never follow a symbolic link, only remove this server's own `kora-<name>` system account, only delete the program copy in Kora's program folder, hand a kept folder back only to a plain `uid:gid`, never close the SSH port, and with ufw only remove rules carrying Kora's label.
  - Kora's records, the schedule setting and `secrets.env` are written without following planted symbolic links.
- **The helper is limited to what the panel needs**: installs only into a new or empty folder in your home folder or `/home/kora`, no other catalogue, only newer Kora versions; git run as root no longer reads your git settings or runs hooks (your GitHub sign-in still works). The guide now says plainly that the helper gives your account the same power as passwordless `sudo` for Kora's actions.
- **The panel's session cookie is useless on its own.** Browsers send cookies to every port of a host, so txAdmin or a FiveM resource listening on the game port could see the panel's cookie and use it. Every request must now also carry a per-session secret the page keeps for its own address and port. The cookie is named per port (`kora_session_<port>`).
- The panel refuses requests that came through a proxy or `tailscale serve` (they make every visitor look like the computer itself).
- The panel refuses to listen on a public address or on every interface (`0.0.0.0`, `::`) unless you add `--allow-public`: it is plain HTTP.
- Remote sign-in failures are counted per address (IPv6 per /64), so someone guessing tokens only locks themselves out. Remote sessions are limited to 8 live views each and 64 together; server status is read once for every open page; the fleet list is reused for 5 seconds.
- A remote session can only follow a job its role could have started, and "already running" answers no longer show the job's id. The fleet and other panels' servers (the hub) are local only, as documented. From another computer the alert webhook must be a public internet address.
- **FXServer downloads can no longer write outside their folder through symlinks** in the Linux archive. Resources installed from a repository subfolder refuse symlinks, catalogue paths with a drive letter or `:` are refused and every clone or copy must land inside `resources`, and resource names `.`, `..` and names starting with `-` are refused.
- Windows: Kora's folders in `%ProgramData%` must belong to administrators; a folder someone else created first is refused.
- `kora self-update` no longer carries setuid/setgid bits over, and private files can no longer keep looser permissions from a file left behind by an earlier run.
- `kora delete` takes the server's name and ports from your server list, not from files the server's account can change; a folder that is not in the list gets no system-wide changes. Delete only drops databases Kora created, never another server's or MariaDB's own.
- Root never writes into a server folder through a planted symbolic link, hard link or FIFO (update log and lock, state files, configs, secrets), and an install refuses a symbolic link another account could have planted on the way to the server folder. A re-run install always runs the server as its own `kora-<name>` account.
- Older root auto-update jobs that run a program other accounts can change are repaired automatically the next time Kora runs as root.
- A program on the panel's own computer can no longer sign in through the computer's Tailscale address and be taken for its owner.
- Alert webhook addresses set from another computer are read the way they are sent (a backslash can no longer hide the real host), more internal IPv6 forms are refused, and the address is checked again before every send.
- A cookie planted by another service on the same host no longer locks you out; wrong user tokens on ordinary requests count towards that address's lockout; the session secret in a URL is only accepted on live streams.

## [0.5.1] - 2026-11-16

### Changed
- **The resource list comes straight from GitHub.** Kora reads the resource catalog from the docs repository's `catalog/resources.json` on raw.githubusercontent.com instead of the docs website, so a change to the list is live as soon as it is merged (no wait for the docs site to rebuild). Without internet it still uses the copy built into the program. `--catalog` / `KORA_CATALOG` still override it.
- `sd-lib` is no longer offered. The catalog keeps published entries for history, so it is marked `"retired": true` there; Kora skips retired resources (servers that already have one keep it).

## [0.5.0] - 2026-11-16

0.4.3 was prepared but never published; its fix is included here.

### Added
- **Paid assets from the Cfx.re Portal.** Install escrowed assets from the `.zip` files you download at portal.cfx.re (Assets, Granted Assets): `kora add-asset <server> <zip>...` (`--dry-run`, `--yes`) or the panel's new **Paid assets** tab, where you upload a zip or pick a recent one from your Downloads folder. Kora checks the zip first (escrow `.fxap` files, unsafe paths, zip bombs, oversized content, name clashes), installs each asset into `resources/[paid]` without renaming anything, adds `ensure` to `server.cfg`, and keeps backups when you replace an asset with a newer zip. The panel explains the licence errors in the server console ("You lack the required entitlement", "Failed to verify protected resource") in plain words. Updates leave paid assets alone.
- **The install scripts start Kora Panel for you.** After `install.sh` / `install.ps1`, Kora Panel is set up as a background service (it comes back after a restart), opens in your browser and the sign-in link is printed. On a server with no screen you get the `ssh -L` command to run on your own computer, filled in with the server's address. `KORA_NO_LAUNCH=1` (Windows: `-NoLaunch`) only installs the program.
- `kora panel --setup` does the same without asking questions. `kora install --fix-permissions` lets the background service through private home folders (Linux).

### Changed
- **Linux: servers go in your home folder by default, not root's.** The suggested folder is `~/kora/<name>` for the person who ran Kora, also when Kora asks for `sudo`. Logged in as root, Kora suggests `/home/kora/<name>` and creates a normal `kora` account that owns it. If the background service cannot get into a private home folder, the wizard offers to let it pass through (`chmod o+x` on exactly those folders) or to use `/home/kora/<name>`. Kora decides the folder and your settings folder before asking for `sudo`, so your server list and saved answers stay yours, and in development mode GitHub commands use your own GitHub sign-in.
- `kora panel` picks the next free port (47121-47140) when 47120 is taken, and a second `kora panel` shows the link of the one already running instead of failing. Opening the browser works on macOS and under `sudo` on Linux, and says so when it fails.
- **Updates are safer.**
  - One update runs per server at a time. If another one is already running, the panel and the CLI say so ("Another update is running for this server") and the nightly run simply skips.
  - On production servers Kora never moves a resource away from commits of your own and never downgrades it.
  - If an update stops part way, every resource it did not reach is listed, and the server is only restarted if your restart choice allows it.
  - Rolling back refuses to throw away changed files unless you choose "Roll back anyway" (`--force`).
  - Automatic updates save snapshots you can undo from the panel or with `kora update --rollback` (the newest 20 are kept), and a version that had to be rolled back is not retried every night until a newer one appears.
  - Updating applies exactly the versions you previewed. The panel warns when something needs a look, says when the built-in release list was used, and shows short commit ids. Panel updates default to restarting only when nobody is playing.

### Fixed
- **Development installs reuse the fork you already have**, even if GitHub named it `name-1`, you renamed it, or it is a fork of another fork. Before, Kora forked again, waited five minutes on the wrong name, marked the resource as failed, and did it again on every run. Progress now says whether each fork was created or reused ("created 2 forks, reused 14 you already had"); the same applies when adding resources.
- The background panel no longer waits on a question nobody can see when it starts in a hidden window (Windows, or a computer reached over SSH).
- Discord update notifications are no longer sent twice; the update badge counts items that need attention and refreshes after single-resource updates.
- `sd-lib` is no longer in the built-in resource list.
- **The panel's install wizard no longer looks stuck while it waits.** The GitHub step shows a spinner while it loads your accounts and organisations (the list is disabled and says "Loading your accounts..."), and "Checking GitHub..." spins too. After you click "Install the server" the button shows "Starting..." with a note that Kora is checking ports, GitHub and permissions, and once the install has started the page shows "Getting ready" until the first step appears, instead of an empty progress bar.

## [0.4.2] - 2026-11-16

### Changed
- **`kora panel` on a server with no screen now offers to run in the background.** A panel started in an SSH window stopped as soon as the window closed. Now `kora panel` asks "Keep it running in the background instead? [Y/n]" (Enter = yes), sets up the background service and prints the sign-in link and the `ssh -L` tunnel line. Running `kora panel` again shows the link of the running background panel instead of starting a second one. Scripts and pipes are never asked. New `--foreground` runs it in the current window only.

## [0.4.1] - 2026-11-16

### Added
- **Your Cfx.re account becomes the txAdmin admin.** The terminal wizard and the panel's install wizard ask for your Cfx.re username (after the licence key), look the account up on forum.cfx.re (only the name is sent) and set it up as the txAdmin admin with admin rights in `permissions.cfg`, so you can sign in to txAdmin with your Cfx.re account straight away. Leave it empty to keep the generated local admin.
- `kora install --cfx-user <NAME>` does the same from the command line. New panel route `POST /api/v1/checks/cfx`.
- **Check one resource at a time.** On the panel's Updates tab each resource has a Check button that looks at just that resource, and shows when it was last checked.

### Changed
- **Checking for updates is much faster.** Resources are fetched in parallel (up to 6 at once) and, for development servers, one request to the KoraFiveM organisation on GitHub lets Kora skip repositories that have not changed. Anything uncertain is still fetched, so an update is never missed (the hint can be up to a minute old; set `KORA_NO_ORG_HINT` to turn it off). In our test, checking 36 resources went from about 17-29 s to 3-13 s, and about 0.6 s when nothing had changed.
- Applying an update right after a check no longer fetches the same resources again.
- Actions the panel cannot do for a server that was added from an existing folder now say so in plain words instead of showing a file error.

### Fixed
- The panel's website and install scripts now point to `kora panel` as the next step after installing.
- **Deleting a server in the panel when the database needs its admin password.** The delete used to fail with a message about an environment variable and `kora delete`. The panel now says what happened in plain words, asks for the database administrator password (used once, never stored), and offers to keep the database and finish.

## [0.4.0] - 2026-11-16

### Added
- **Kora Panel:** `kora panel` opens a local web panel that installs, manages and updates servers in your browser (servers list, start/stop/restart, console, logs, updates with preview, snapshots and rollback, resources, schedule, Discord webhook, ports, GitHub (issues and pull requests), delete, the install wizard, and Settings with self-update, the Kora app and uninstall). `--port` (default 47120), `--no-open`. Guide: `docs/panel.md`.
- Headless use: over SSH or without a desktop the panel prints the sign-in link and an `ssh -L` tunnel hint (also with `--no-open`).
- Panel API (`/api/v1`): every operation is one `kora_core::api::Op`, served through the shared `Backend` trait, with jobs and server-sent events for slow operations, structured errors (`needs_elevation` carries the command to run) and write-only secrets.
- Parity tests: every CLI subcommand and flag, every manager action and wizard screen must map to an operation (or be listed as CLI-only or removed), every operation must have a handler, and the HTTP handlers are tested against a fake backend. A golden file of `kora --help` stops flags disappearing silently.
- `kora install --hostname <TEXT>` and `--game-build <N>`: the public server name and a pinned game build, as real flags (before they could only come from a setup code). The hostname defaults to the server name.
- Privileged helper: `kora helper install|uninstall|status|serve` (Linux), so the panel can run administrator actions without asking each time. It accepts only Kora's existing privileged actions and only from the installing user.
- Monitoring tab: server up/down, players, CPU and memory, disk, ports and the update badge, with about 24 hours of history; alerts as a panel banner, a Discord webhook or a generic webhook.
- Opt-in background panel service: `kora panel --install-service`, `--remove-service`, `--print-link`.
- Remote access: `kora panel --bind <ip> --remote`, panel users with `kora panel user add|list|remove`, roles viewer / operator / admin.
- Tailscale identity for remote sign-in (checked with the local tailscaled, never from request headers).
- Delete, uninstall and console input are off for remote sessions by default and need the server name typed.
- Hub: any panel can list and manage other panels as nodes (bearer tokens; nodes advertise version and supported operations).

### Changed
- The terminal manager and the CLI now use the same shared engine interface as the panel (`kora_core::api`); the start/stop behaviour of `kora start|stop|restart` and the manager no longer differs.
- Minimum supported Rust version is 1.93.
- CI builds, lints, tests and audits (`npm audit --omit=dev --audit-level=high`) the panel UI; releases embed it. Built UI files are not committed. `cargo build` without Node still works and `kora panel` shows a "UI not built" page.
- Signing docs: SignPath is no longer mentioned. The Windows `.exe` is unsigned, SmartScreen may warn, the install scripts verify SHA-256 and the manual `.sha256` check is documented. The privacy line now says Kora contacts nothing unless you ask it to.

### Removed
- Setup codes: `kora install --code` and `kora add-resources --code` still parse but stop at once with exit code 2 and a message pointing to the flags, `--resource` and `kora panel`. The wizard no longer has a setup-code screen.
- macOS code and wording (Kora supports Linux and Windows).

### Security
- The panel listens on `127.0.0.1` only by default, signs you in with a random one-time token exchanged for an `HttpOnly; SameSite=Strict` cookie, rate-limits wrong tokens, checks the `Host` header (DNS rebinding) and `Origin` plus `X-Kora` on changing requests (CSRF), and sends a strict Content-Security-Policy, `nosniff` and `no-referrer`. Licence keys, database passwords and webhook addresses are never returned to the browser.
- The panel UI is installed with `npm ci --ignore-scripts` from a committed lockfile and audited in CI.

### Fixed

- Downloads no longer fail when a signal (for example a child process ending) interrupts a read; the read is retried.
- Monitoring reads a server's `players.json` reply even when the server closes the connection early.

## [0.3.4] - 2026-11-15

### Added
- Change a server's game and txAdmin ports from the manager: select the server while it is stopped, press `m` (or "Change ports"), type the new ports and press Enter. Kora checks they are free and not used by another Kora server, then updates `server.cfg`, the txAdmin port in the service environment (Linux), `kora-server.json` and the server list. Firewall rules are not changed: open the new ports and close the old ones yourself.

## [0.3.3] - 2026-11-15

### Fixed
- The live catalog is no longer rejected because of its `bridges` / `creators` groups: Add resources showed "unknown variant `bridges`", fell back to an old bundled catalog and listed 0 compatibility resources, bridges and creators. Unknown group names are now tolerated (shown as "other"); an unknown `kind` or `type` is still refused, now naming the resource and field and asking you to update the installer.
- The bundled offline catalog is refreshed to the current schema-v3 catalog (compatibility resources and bridges included).

## [0.3.2] - 2026-11-15

### Added
- Development installs adopt forks that already exist in your account or organisation and bring them up to date with upstream: a fork that is only behind is fast-forwarded on GitHub (`gh repo sync`, never `--force`); a fork with its own commits (ahead or diverged) is not touched. The install result and `install.log` list those forks in plain language with the safe fix (`git fetch upstream && git merge upstream/<branch>`, then push) and say nothing was changed on them, plus counts of brought up to date / already current / left alone. Foreign repositories with the same name are still refused.
- Fork syncing is paced by GitHub API calls (batches, pauses, backoff on rate limits); results are saved in the resumable fork state.
- `kora update` (development) fast-forwards behind-only forks on GitHub first when `gh` is signed in, and lists forks it left alone.

### Changed
- `kora update`: a resource whose checkout has commits of its own and can no longer fast-forward is reported as `diverged` and skipped; the other resources are still updated (before, it aborted the run and rolled back).

### Fixed
- `kora install ... --github-owner NAME` (the website one-liner) no longer demands that `gh` is already installed and signed in. The name is checked for a legal GitHub login straight away; if `gh` is ready the owner is verified up front as before, otherwise the install sets `gh` up, signs you in in the browser, and verifies the owner right after that, before anything is forked (an unknown owner stops the install with the list of valid owners; nothing is forked and your answers are saved).

## [0.3.1] - 2026-11-15

Visual-only release: the terminal screens have a new look. Behaviour and key bindings are unchanged.

### Added
- `NO_COLOR` (set and non-empty) and `TERM=dumb` turn colour off completely; state is then carried by bold, reverse and dim text and by symbols, never by colour alone.
- ASCII fallback for all symbols and borders (non-UTF-8 locale, the legacy Windows console, or `KORA_ASCII=1`).

### Changed
- TUI redesign: one central theme with the Kora lime accent (#95ff00), with 256-colour and 16-colour fallbacks (bright green in 16 colours).
- Rounded frame, `KORA` wordmark header, step tabs and gauges, status chips with symbols (`[ ✓ OK ]`, `[ ! WARN ]`, `[ ✗ FAIL ]`), and key-hint chips in the footer (hints that do not fit are dropped).
- Review is a two-column summary.
- Progress shows a per-step icon (pending, spinner, done, failed) and a scrollbar.
- Add resources has section tabs and dependency badges (`<needs ox-lib>`).
- Manager has a list header.
- Screens smaller than 80x24 show a "Terminal too small" panel instead of a broken layout.

## [0.3.0] - 2026-11-15

### Added
- Catalog schema v3 (v2 still works): compat, bridge and creator resources, creators, tag-less releases, `subpath` installs. Bridge and creator resources are always pinned clones, never forked, even in development mode.
- New installs use the folders `[cfx-default]`, `[compat]`, `[kora]`, `[bridges]`, `[creators]/[<creator>]`, plus empty `[addons]` and `[streamedassets]`; `server.cfg` ensures folders, not single resources. Compat, bridge and creator resources are never installed unless chosen or needed.
- Add resources to an installed server: `kora add-resources <server> --code CODE | --resource ID`, and `Add resources (g)` in the manager (browse compat / bridges / creators, with dependencies kept). Existing servers are not rearranged; missing `ensure` lines are appended.
- Manager: `Report an issue (b)` for both modes and `Open pull request (p)` for development servers, through `gh`. If `gh` is not signed in, Kora offers the browser sign-in first; issue and pull request text never contains paths, host names, ports, keys or passwords.

### Changed
- Catalog v3 needs this version: an older Kora does not understand it. Once the catalog moves to schema v3, v0.2.x installs stop reading it, so update with `kora self-update`.
- Smoke tests (Linux and Windows) now expect `server.cfg` to ensure folders instead of single resources.

## [0.2.3] - 2026-11-15

### Added
- Live console in `kora manager` (`w`): follows the server's output and sends commands through a small `kora_console` helper resource (`.kora/console.in`). New installs include it; existing servers turn it on with F2. `kora service-host` now also writes the server output to `.kora/console.log`.
- Development installs sign in to GitHub in the browser during setup when `gh` was just installed (one-time code shown, page opened) instead of failing with "run gh auth login".
- Answers of a failed install are saved to `<kora home>/last-install.json` (never the license key or database password) and restored on the next run.
- Confirm-password box on the database admin login; the login is tested before the install continues.
- The review screen is a two-column summary and states exactly which firewall ports will be opened.

### Fixed
- Windows service start failing with error 1053: `service-host` now answers the service manager first and logs any start-up failure to `.kora/service-host.log`.

### Changed
- Accent colour is Kora lime (#95ff00); intro screens have spaced paragraphs.

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
