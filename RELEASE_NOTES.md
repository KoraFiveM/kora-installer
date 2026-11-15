## Kora Installer v0.3.1

Install and manage a Kora FiveM server from one program: `kora install` (guided wizard or flags).

### Install
Linux:
```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/download/v0.3.1/install.sh | sh
```
Windows (PowerShell):
```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/download/v0.3.1/install.ps1 | iex
```
Then run `kora install`. Or download the program below and check it yourself.

### New in 0.3.1
- **A new look for the terminal screens.** One theme with the Kora lime accent, a rounded frame, a `KORA` header, step tabs and progress gauges, status chips with symbols, and key hints in the footer. The review is a two-column summary; progress shows an icon per step and a scrollbar; "Add resources" has section tabs and shows dependencies as badges; the manager list has a header.
- **Terminal too small?** Below 80x24 you now get a clear "Terminal too small" panel instead of a broken layout. Make the window bigger.
- **Visual changes only.** Behaviour and key bindings are unchanged.
- **Colour fallbacks.** 256-colour and 16-colour terminals get the closest green. Set `NO_COLOR=1` (or `TERM=dumb`) for no colour at all; state is then shown with bold, reverse and symbols.
- **ASCII fallback.** Symbols and borders switch to plain ASCII on a non-UTF-8 locale and on the legacy Windows console. Set `KORA_ASCII=1` to force it.

### Earlier: 0.3.0
- **You need 0.3.0 or newer for the new catalog (schema v3).** v0.2.x installs keep working only until the catalog moves to schema v3: run `kora self-update`, or use the install line above.
- **Add resources to an existing server:** `kora add-resources <server> --code CODE` or `--resource ID`, or press `g` in `kora manager`. Dependencies are added with them.
- **Tidier folder layout for new servers** (`[cfx-default]`, `[compat]`, `[kora]`, `[bridges]`, `[creators]`, plus empty `[addons]` and `[streamedassets]`); `server.cfg` starts whole folders. Existing servers are not rearranged.
- **Report an issue** (`b`) and **open a pull request** (`p`, development servers) from the manager, through the GitHub CLI; no paths, host names, ports, keys or passwords are sent.
- **Third-party resources are always pinned** to an exact commit, never forked, even in development mode.

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
- The new look is lightly tested in the Windows console; colours and symbols may render differently there. If it looks wrong, set `KORA_ASCII=1` and/or `NO_COLOR=1`, and please report it.
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
