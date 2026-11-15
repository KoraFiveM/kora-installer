## Kora Installer v0.2.3

Install and manage a Kora FiveM server from one program: `kora install` (guided wizard or flags).

### Install
Linux:
```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/download/v0.2.3/install.sh | sh
```
Windows (PowerShell):
```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/download/v0.2.3/install.ps1 | iex
```
Then run `kora install`. Or download the program below and check it yourself.

### New in 0.2.3
- **Live console.** In `kora manager`, press `w` on a server to watch its console as it happens and type commands into it. Servers you install now get it automatically; for an existing server press F2 on that screen once and restart the server.
- **GitHub sign-in is part of setup.** If the GitHub CLI had to be installed for a development server, Kora now opens your browser to sign in (it shows the one-time code) instead of stopping with an error.
- **Fixes the Windows "service failed to start (1053)" dead end.** If the server could not start, the service now says why in `.kora\service-host.log` instead of failing silently.
- **A failed install no longer makes you start over.** Your answers are saved (never the license key or database password) and filled in next time; Kora continues where it stopped.
- **Safer database step.** The admin password is typed twice, and the login is tested before the install continues.
- **Clearer review screen.** A tidy two-column summary, and a plain statement of exactly which firewall ports will be opened.
- **Kora lime** colour throughout, and easier-to-read intro screens.

### Earlier: 0.2.2
- **Update your server on your terms.** `kora update <server>` shows what would change, saves a snapshot, applies the update and puts everything back automatically if it goes wrong. `kora update <server> --preview` only looks; `--rollback <snapshot>` undoes an update later; `--list-snapshots` shows what you can go back to.
- **Discord notifications.** `kora webhook set <server>` sends the report of each automatic update to a Discord channel (`kora webhook test <server>` sends a test message).
- **You choose when automatic updates run.** `kora schedule set <server> --daily 04:00` or `--weekly sun 04:00`. It now runs as you, not as the system account, so private GitHub forks can be updated.
- **Kora is now an app.** After installing, Windows gets a "Kora" Start Menu shortcut (with the fox icon, plus an optional Desktop shortcut) and `kora` works from any terminal; Linux gets a `kora` command and a launcher. Remove it with `kora app uninstall`; your servers are never touched.
- **Much faster Windows database setup.** MariaDB (11.4.13) now downloads from the official mirrors, about 300x faster than before at its slowest, with a progress display and a checksum check before anything is installed.
- **Safer.** An internal security review found no serious problems, but fixed seven smaller ones, including: downloads can no longer be redirected to plain http, a file planted in your Downloads folder can no longer be run with administrator rights, and your licence key is no longer passed on the command line when Kora asks for permission. Details in the changelog.
- **Kora updates itself.** `kora self-update` (or `--check` to just look) fetches the newest release, verifies its checksum and keeps a backup. Kora also tells you when a new version is out.
- **A screen for all your servers.** `kora manager`, or just `kora` once you have a server, lists your servers and lets you start, stop, update and look at logs without remembering commands.

### What works
- Guided install (production and development), setup code from the website, resource dependencies, free-space and port checks, uninstall.
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
From 0.2.1: run the install one-liner above (or download the program) and run `kora install` or `kora app install` to get the app shortcut and PATH entry. Existing servers keep working. Servers installed by 0.2.1 or earlier keep their old update schedule: run `kora schedule set <server> --daily 04:00` to switch it to run as you. If you used v0.1.0 / v0.2.0, either run `kora delete <folder>` for the old install, or re-run `kora install` to resume it.

Full list of changes: [CHANGELOG.md](https://github.com/KoraFiveM/kora-installer/blob/main/CHANGELOG.md)
