# Changelog

All notable changes to Kora Installer. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [SemVer](https://semver.org/).

## [Unreleased]

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
