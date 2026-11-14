## Kora Installer v0.2.1

Install and manage a Kora FiveM server from one program: `kora install` (guided wizard or flags).

### Install
Linux:
```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/download/v0.2.1/install.sh | sh
```
Windows (PowerShell):
```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/download/v0.2.1/install.ps1 | iex
```
Then run `kora install`. Or download the program below and check it yourself.

### New in 0.2.1
- **No more "run as administrator" instructions.** On Windows Kora now asks Windows for permission itself (one prompt), and the window stays open until you press a key. On Linux it uses `sudo` when your choices need it.
- **Your server starts when the install finishes**, and the last screen shows it running with Open txAdmin, Copy connect address and Stop server buttons. A rejected licence key or a crash on start is reported plainly.
- **Development mode:** sign in to GitHub in the wizard and pick which account or organisation your forks go into (or create a new organisation).
- Earlier in the 0.2 line: Paste buttons, clickable log/folder buttons, free-port suggestions, Windows MariaDB install.

### What works
- Guided install (production and development), setup code from the website, resource dependencies, free-space and port checks, uninstall.
- **Linux:** tested end to end.
- **Windows:** covered by CI smoke tests (install script, production install, dev dry-run, service and firewall steps); real-machine runs have started and are not finished. Please report problems.

### Known gaps
- `kora manager` (the screen for managing your servers later) and the Start Menu / `kora` app install are not released yet.
- Manual update with preview and rollback is not built yet.
- The Discord webhook is not sent yet.
- Production update path is missing: production servers move to newer releases manually.
- The Windows `.exe` is **unsigned** (SignPath application pending), so SmartScreen will warn ("More info" -> "Run anyway").

### Verify your download
Each program has a `.sha256` file next to it. The install scripts check it automatically. By hand:
```sh
sha256sum -c kora-linux-x86_64.sha256
```
```powershell
(Get-FileHash kora-windows-x86_64.exe -Algorithm SHA256).Hash.ToLower()   # compare with the value in kora-windows-x86_64.exe.sha256
```

### Upgrading from v0.1.0 / v0.2.0
If you used an earlier build, either run `kora delete <folder>` for the old install, or re-run `kora install` to resume it.

Full list of changes: [CHANGELOG.md](https://github.com/KoraFiveM/kora-installer/blob/main/CHANGELOG.md)
