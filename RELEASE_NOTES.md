## Kora Installer v0.3.3

Install and manage a Kora FiveM server from one program: `kora install` (guided wizard or flags).

### Install
Linux:
```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/download/v0.3.3/install.sh | sh
```
Windows (PowerShell):
```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/download/v0.3.3/install.ps1 | iex
```
Then run `kora install`. Or download the program below and check it yourself.

### New in 0.3.3
- **Fixes the Add resources screen showing "0 available".** Installers 0.3.0 to 0.3.2 could not read the live catalog once it contained bridge entries (`group: "bridges"` was an unknown value). They silently fell back to the old bundled catalog, and Compatibility, Bridges and Creators showed "0 available" with the message "Used the catalog bundled with this installer (catalog parse error: unknown variant `bridges` ...)".
- **What to do:** run `kora self-update`, or re-run the install line above. Until you do, the Add resources screen shows only the bundled list.
- **Newer catalogs no longer break the installer.** The `bridges` and `creators` groups are accepted, and any unknown future group is shown as `other`. An unrecognised `kind` or `type` is still refused on purpose (no guessing), but the error now names the resource and the field and tells you to update the Kora installer.
- **The bundled offline catalog is refreshed** to the current live catalog: the compatibility resource plus four bridges (`community_bridge`, `sd_lib`, `loaf_lib`, `Newb_Bridge`). Bridges are third-party and not supported by Kora.

### Earlier: 0.3.2
- **Development installs adopt your existing forks.** An existing fork of a Kora repository is reused and brought up to date: a fork that is only behind is fast-forwarded on GitHub (`gh repo sync`, never `--force`); a fork with its own commits is left alone and listed with the safe fix (`git fetch upstream && git merge upstream/<branch>`, then push). Same-name repositories that are not Kora forks are still refused. `kora update` (development) does the same.
- **`kora update` no longer aborts on a diverged resource**: it is reported as `diverged` and skipped, and the rest are still updated.
- **The website one-liner with `--github-owner NAME` works without `gh` ready**: the install sets it up, signs you in, and verifies the owner before anything is forked.

### Earlier: 0.3.1
- **A new look for the terminal screens** (Kora lime theme, rounded frame, step tabs, gauges, status chips, footer key hints), with a "Terminal too small" panel below 80x24. Visual changes only; behaviour and key bindings are unchanged.
- **Fallbacks:** 256- and 16-colour terminals get the closest green; `NO_COLOR=1` / `TERM=dumb` turn colour off; `KORA_ASCII=1` (or a non-UTF-8 locale / legacy Windows console) switches to plain ASCII.

### Earlier: 0.3.0
- **You need 0.3.0 or newer for the new catalog (schema v3):** run `kora self-update`, or use the install line above.
- **Add resources to an existing server:** `kora add-resources <server> --resource ID`, or `g` in `kora manager`. Tidier folder layout for new servers; existing servers are not rearranged.
- **Report an issue** (`b`) and **open a pull request** (`p`, development servers) from the manager, through the GitHub CLI; no paths, host names, ports, keys or passwords are sent. Third-party resources are always pinned to an exact commit, never forked.

### Earlier: 0.2.3
- **Live console** in `kora manager` (`w`), and **GitHub sign-in as part of setup** for development servers.
- Fixes the Windows service "failed to start (1053)" dead end; a failed install keeps your answers (never the license key or database password); the database admin password is typed twice and tested; clearer review screen.

### Earlier: 0.2.2
- `kora update <server>` with preview, snapshot and rollback; Discord notifications; update schedule you choose; Kora as an app (shortcut, `kora` command); faster Windows database setup; `kora self-update`; `kora manager`; security fixes. Details in the changelog.

### What works
- Guided install (production and development), resource dependencies, free-space and port checks, uninstall.
- Add resources to an existing server, report an issue and open a pull request from the manager.
- Manual update with preview, snapshot and rollback, Discord webhook, schedule, and the app install (shortcut, PATH, `kora` command).
- **Linux:** tested end to end, including automated end-to-end tests in CI.
- **Windows:** covered by CI smoke tests (install script, production install, dev dry-run, service and firewall steps, app install); real-machine runs have started and are not finished. Please report problems.

### Known gaps
- The Creators list in Add resources is empty at the moment; there is nothing to show there yet.
- **Fork syncing and the first-run `gh` flow are not verified against real GitHub or a real Windows machine yet** (CI and mocks only). If `gh repo sync` refuses for a fork (for example a protected branch or no permission), that fork is left untouched and listed as left alone, with the safe fix above; the install carries on. Please report problems.
- Real Windows runs of the new features (app install, schedule as the installing user) are covered by CI only so far.
- The new look is lightly tested in the Windows console; colours and symbols may render differently there. If it looks wrong, set `KORA_ASCII=1` and/or `NO_COLOR=1`, and please report it.
- The MariaDB root password is briefly visible to administrators in the process list while the database is created (a limit of the MariaDB tool).
- The Windows `.exe` is **unsigned**, so SmartScreen may warn ("More info" -> "Run anyway").

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
