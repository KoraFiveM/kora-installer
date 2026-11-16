## Kora Installer v0.4.0 - Kora Panel

Install and manage a Kora FiveM server from your browser: run `kora panel`.

### Install
Linux:
```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/download/v0.4.0/install.sh | sh
```
Windows (PowerShell):
```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/download/v0.4.0/install.ps1 | iex
```
Already have Kora? Run `kora self-update`.

### New in 0.4.0
- **Kora Panel.** Run `kora panel`; your browser opens a local page where you can install a server (the same wizard as `kora install`), start, stop and restart it, watch the console and logs, update with preview, snapshot and rollback, add resources, change the schedule, ports and Discord webhook, and delete or uninstall. Everything the command line and the terminal manager do is in the panel.
- **Safe by default.** The panel only listens on your own computer (`127.0.0.1`, port 47120; `--port` changes it) and signs you in with a one-time link. Nothing is exposed to your network.
- **On a server with no screen** (SSH): `kora panel --no-open` prints the link and an `ssh -L` command to open it from your own computer.
- **Administrator actions:** the panel runs as you. When something needs administrator rights it shows the exact command to run.
- **Scripted installs** gain `--hostname` and `--game-build`.
- <!-- VERIFY: wave-3 features --> Also in this release: a privileged helper (`kora helper`), a Monitoring tab with alerts and an optional background panel service, remote access with users and roles, and a hub for several machines. See the guide.

How to try it: `kora panel`, then follow the link. Guide: [docs/panel.md](https://github.com/KoraFiveM/kora-installer/blob/main/docs/panel.md).

### Removed
- **Setup codes.** `kora install --code` and `kora add-resources --code` now stop with a message (exit code 2). Instead use `kora panel`, or flags: `kora install --name ... --hostname ... --game-build ... --resources ...` and `kora add-resources <server> --resource ID`.
- macOS wording and code (Kora supports Linux and Windows).

### Upgrading
- Run `kora self-update`, or the install line above. Existing servers keep working; nothing is rearranged.
- Scripts that used `--code` need the flags above.
- Building from source now needs Rust 1.93 or newer, and Node.js to include the panel UI (`cd web && npm ci --ignore-scripts && npm run build`). Without it `kora panel` shows a "UI not built" page.

### Verify your download
The Windows `.exe` is **unsigned**, so SmartScreen may warn ("More info", then "Run anyway"). Each program has a `.sha256` file next to it; the install scripts check it automatically. By hand:
```sh
sha256sum -c kora-linux-x86_64.sha256
```
```powershell
(Get-FileHash kora-windows-x86_64.exe -Algorithm SHA256).Hash.ToLower()   # compare with the value in kora-windows-x86_64.exe.sha256
```

### Known gaps
- Linux is tested end to end; Windows is covered by CI smoke tests and real-machine runs are still in progress. Please report problems.
- The panel's Windows behaviour (browser opening, administrator prompts) has had less real-machine testing than Linux.

Earlier releases: see [CHANGELOG.md](https://github.com/KoraFiveM/kora-installer/blob/main/CHANGELOG.md).
