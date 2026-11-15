## Kora Installer v0.3.0

Install and manage a Kora FiveM server from one program: `kora install` (guided wizard or flags).

### Install
Linux:
```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/download/v0.3.0/install.sh | sh
```
Windows (PowerShell):
```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/download/v0.3.0/install.ps1 | iex
```
Then run `kora install`. Or download the program below and check it yourself.

### New in 0.3.0
- **You need this version for the new catalog.** The resource catalog is moving to a new format (schema v3). Kora 0.3.0 reads it; older versions do not. v0.2.x installs keep working only until the catalog moves to schema v3, so please update now: run `kora self-update`, or use the install line above.
- **Add resources to a server you already have.** `kora add-resources <server> --code CODE` (a code from the website) or `--resource ID`. In `kora manager`, press `g` on a server to browse compatibility resources, bridges and creator resources. Anything a resource depends on is added with it.
- **A tidier folder layout for new servers.** Folders are `[cfx-default]`, `[compat]`, `[kora]`, `[bridges]` and `[creators]/[<creator>]`, plus empty `[addons]` and `[streamedassets]` for your own things. `server.cfg` now starts whole folders instead of listing each resource. Compatibility, bridge and creator resources are only installed when you choose them or something needs them.
- **Existing servers are not rearranged.** Your folders and `server.cfg` stay as they are. Adding resources only appends the missing `ensure` lines.
- **Report an issue from the manager.** Press `b` on a server to send an issue to the resource's repository through the GitHub CLI. It contains only the Kora version, your system, the mode, the resource and its version. No paths, host names, ports, keys or passwords.
- **Open a pull request from the manager** (development servers). Press `p` to push your fork's branch and open a pull request to the original repository. If the GitHub CLI is not signed in, Kora offers to sign you in first.
- **Third-party resources are always pinned.** Bridge and creator resources are installed at an exact commit, never as a fork, even in development mode, so they cannot change under you. Releases without a tag are pinned by commit.

### Earlier: 0.2.3
- **Live console** in `kora manager` (`w`), and **GitHub sign-in as part of setup** for development servers.
- Fixes the Windows service "failed to start (1053)" dead end; a failed install keeps your answers (never the license key or database password); the database admin password is typed twice and tested; clearer review screen.

### Earlier: 0.2.2
- `kora update <server>` with preview, snapshot and rollback; Discord notifications; update schedule you choose; Kora as an app (shortcut, `kora` command); faster Windows database setup; `kora self-update`; `kora manager`; security fixes. Details in the changelog.

### What works
- Guided install (production and development), setup code from the website, resource dependencies, free-space and port checks, uninstall.
- Add resources to an existing server, report an issue and open a pull request from the manager.
- Manual update with preview, snapshot and rollback, Discord webhook, schedule, and the app install (shortcut, PATH, `kora` command).
- **Linux:** tested end to end, including automated end-to-end tests in CI.
- **Windows:** covered by CI smoke tests (install script, production install, dev dry-run, service and firewall steps, app install); real-machine runs have started and are not finished. Please report problems.

### Known gaps
- Real Windows runs of the new features (app install, schedule as the installing user) are covered by CI only so far.
- The MariaDB root password is briefly visible to administrators in the process list while the database is created (a limit of the MariaDB tool).
- The Windows `.exe` is **unsigned** (SignPath application pending), so SmartScreen will warn ("More info" -> "Run anyway").

### Verify your download
Each program has a `.sha256` file next to it. The install scripts check it automatically. By hand:
```sh
sha256sum -c kora-linux-x86_64.sha256
```
```powershell
(Get-FileHash kora-windows-x86_64.exe -Algorithm SHA256).Hash.ToLower()   # compare with the value in kora-windows-x86_64.exe.sha256
```

### Upgrading
From 0.2.x: run `kora self-update` (or the install one-liner above). **Do this before the catalog moves to schema v3**: after that, v0.2.x installs can no longer read the catalog. Existing servers keep working and are not rearranged; use `add-resources` to add to them. Servers installed by 0.2.1 or earlier keep their old update schedule: run `kora schedule set <server> --daily 04:00` to switch it to run as you.

Full list of changes: [CHANGELOG.md](https://github.com/KoraFiveM/kora-installer/blob/main/CHANGELOG.md)
