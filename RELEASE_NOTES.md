## Kora Installer v0.3.2

Install and manage a Kora FiveM server from one program: `kora install` (guided wizard or flags).

### Install
Linux:
```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/download/v0.3.2/install.sh | sh
```
Windows (PowerShell):
```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/download/v0.3.2/install.ps1 | iex
```
Then run `kora install`. Or download the program below and check it yourself.

### New in 0.3.2
- **Development installs adopt your existing forks.** If a fork of a Kora repository already exists in your account or organisation, the install reuses it instead of failing, and brings it up to date with upstream. A fork that is only behind is fast-forwarded on GitHub (`gh repo sync`, never `--force`), so history is never rewritten. A fork with commits of its own (ahead or diverged) is left alone. The install result and `install.log` list those forks in plain language, say nothing was changed on them, and give the safe fix: `git fetch upstream && git merge upstream/<branch>`, then push. Repositories with the same name that are not forks of Kora are still refused.
- **`kora update` (development) does the same.** With `gh` signed in, it fast-forwards behind-only forks first and lists the ones it left alone.
- **`kora update` no longer aborts on a diverged resource.** A resource whose checkout has its own commits and can no longer fast-forward is reported as `diverged` and skipped; the rest are still updated (before, the whole run aborted and rolled back).
- **The website one-liner for development codes works without `gh` ready.** `--github-owner NAME` no longer needs `gh` installed and signed in beforehand. The name is checked straight away; if `gh` is not ready, the install sets it up, signs you in in the browser, and verifies the owner before anything is forked. An unknown owner stops the install with the list of valid owners; nothing is forked and your answers are saved.

### Earlier: 0.3.1
- **A new look for the terminal screens** (Kora lime theme, rounded frame, step tabs, gauges, status chips, footer key hints), with a "Terminal too small" panel below 80x24. Visual changes only; behaviour and key bindings are unchanged.
- **Fallbacks:** 256- and 16-colour terminals get the closest green; `NO_COLOR=1` / `TERM=dumb` turn colour off; `KORA_ASCII=1` (or a non-UTF-8 locale / legacy Windows console) switches to plain ASCII.

### Earlier: 0.3.0
- **You need 0.3.0 or newer for the new catalog (schema v3):** run `kora self-update`, or use the install line above.
- **Add resources to an existing server:** `kora add-resources <server> --code CODE` or `--resource ID`, or `g` in `kora manager`. Tidier folder layout for new servers; existing servers are not rearranged.
- **Report an issue** (`b`) and **open a pull request** (`p`, development servers) from the manager, through the GitHub CLI; no paths, host names, ports, keys or passwords are sent. Third-party resources are always pinned to an exact commit, never forked.

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
- **Fork syncing and the first-run `gh` flow are not verified against real GitHub or a real Windows machine yet** (CI and mocks only). If `gh repo sync` refuses for a fork (for example a protected branch or no permission), that fork is left untouched and listed as left alone, with the safe fix above; the install carries on. Please report problems.
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
