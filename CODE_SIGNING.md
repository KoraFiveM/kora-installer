# Code signing

The Windows `kora.exe` is **not code-signed**, so Windows SmartScreen may warn ("More info", then "Run anyway").

Instead, every release is built from this public repository by the GitHub Actions workflow, and each download has a
SHA-256 file next to it (`.sha256`):

- The install scripts (`install.sh`, `install.ps1`) verify the SHA-256 before running anything.
- To check by hand, compare the hash with the `.sha256` file:

```sh
sha256sum -c kora-linux-x86_64.sha256
```
```powershell
(Get-FileHash kora-windows-x86_64.exe -Algorithm SHA256).Hash.ToLower()   # compare with the .sha256 file
```

Kora changes your system (services, scheduled tasks, firewall rules, registry entry). The installer lists every change
before it happens, and `kora uninstall` removes them. No packers (e.g. UPX) are used.
