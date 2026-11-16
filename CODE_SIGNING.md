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

## Build provenance (who built this file)

A matching SHA-256 only proves the file is the one on the release page. To check that it was built by this
repository's GitHub Actions release workflow (and not uploaded by hand), every release file (`kora-linux-x86_64`,
`kora-windows-x86_64.exe`, `install.sh`, `install.ps1`) has a build provenance attestation, signed with GitHub's
Sigstore-based keyless signing (no Kora key to trust or lose). Releases after v0.5.1 have one (older ones only
have the `.sha256`). Check a download with the [GitHub CLI](https://cli.github.com/):

```sh
gh attestation verify kora-linux-x86_64 --repo KoraFiveM/kora-installer
```
```powershell
gh attestation verify kora-windows-x86_64.exe --repo KoraFiveM/kora-installer
```

It prints the workflow, commit and tag the file was built from, and fails if the file was not built there. The
attestations are also listed on the repository's Attestations page.

## Build pipeline

Every GitHub Action used by the workflows is pinned to a full commit SHA (a tag can be moved to other code; a SHA
cannot), and a CI check fails if one is not.

Kora changes your system (services, scheduled tasks, firewall rules, registry entry). The installer lists every change
before it happens, and `kora uninstall` removes them. No packers (e.g. UPX) are used.
