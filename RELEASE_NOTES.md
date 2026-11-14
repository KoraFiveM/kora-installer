## Kora Installer v0.2.0

Install and manage a Kora FiveM server from one program: `kora install` (guided wizard or flags).

### Install
Linux:
```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/download/v0.2.0/install.sh | sh
```
Windows (PowerShell):
```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/download/v0.2.0/install.ps1 | iex
```
Then run `kora install`. Or download the program below and check it yourself.

### What works
- Guided install (production and development), setup code from the website, resource dependencies, free-space and port checks, uninstall.
- **Linux:** tested end to end.
- **Windows:** covered by CI smoke tests (install script, production install, dev dry-run, service and firewall steps); real-machine runs, including MariaDB, have started but are not finished. Please report problems.

### Known gaps
- `kora manager` (ongoing server management) is not built yet.
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

### Upgrading from v0.1.0
If you used v0.1.0, either run `kora delete <folder>` for the old install, or re-run `kora install` to resume it.

Full list of changes: [CHANGELOG.md](https://github.com/KoraFiveM/kora-installer/blob/main/CHANGELOG.md)
