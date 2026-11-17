# Code signing and release trust

Written 2026-11-17. Everything under "Verified" below was read on the vendors' own pages on that date; what I could not
verify is listed at the end. Prices, regions and rules change: re-check before paying or applying.

## The problem

`kora-windows-x86_64.exe` has no Authenticode signature, so on first run Windows SmartScreen says "Windows protected
your PC" and the non-technical people Kora is for have to find "More info" → "Run anyway". On Windows 11 with Smart
App Control on, an unsigned file with no reputation can be **blocked outright** (Microsoft: Smart App Control "will
block execution of unsigned files unless the file has a positive reputation"). Signing is the largest remaining
barrier to non-technical users on Windows.

**Signing alone does not make the warning vanish.** Microsoft: SmartScreen weighs *publisher reputation* and *file
hash reputation*; a newly signed file "could still show a SmartScreen warning until its hash or publisher certificate
accumulates sufficient evidence of positive reputation", which "can take several weeks and hundreds of clean installs".
What a signature changes: the prompt shows a **verified publisher name** instead of "Unknown publisher", reputation
carries over from release to release (unsigned files start from zero every version), Smart App Control can trust it, and
EDR/antivirus products treat it better. EV certificates **no longer** bypass SmartScreen (Microsoft's own page).

## The options (verified 2026-11-17)

| | SignPath Foundation (free, open source) | Azure Artifact Signing (was "Trusted Signing") | OV code signing certificate (DigiCert, Sectigo, SSL.com, Certum, ...) |
|---|---|---|---|
| Cost | Free | Basic $9.99/month incl. 5,000 signatures, then $0.005 each; Premium $99.99/month incl. 100,000. Billed in full from account creation, not pro-rated; needs a **paid** Azure subscription (no free/trial/sponsored) | About $100-$530/year depending on the CA; the key must be on hardware (token or cloud HSM), sometimes extra |
| Who may apply | Open-source projects that meet the conditions below; the project, not a legal entity | **Organizations** in the USA, Canada, EU, UK, Australia, New Zealand, Japan, South Korea, Singapore, Switzerland, Norway, Israel; **individuals only in the USA and Canada**. Identity validation 1 to 20 business days (a third-party article; Microsoft says "a few business days"), "can't be expedited" | A legal entity or an individual, worldwide; several business days of validation |
| Publisher shown | "SignPath Foundation" (the certificate is issued to the Foundation, not to Kora) | The validated legal name of the organization or individual (cannot be customised) | The validated legal name |
| Key custody | SignPath's HSM; you never hold a key | Microsoft's HSM (FIPS 140-3 L3), certificates valid 3 days, auto-renewed | Token / cloud HSM you hold; certificates now max 460 days (CA/B Forum CSC-31, from 2026-03-01), so re-issue about yearly |
| CI integration | `SignPath/github-action-submit-signing-request` (this repo: `sign-windows.yml`) | Azure's GitHub action (published as `azure/trusted-signing-action`; check its current name after the rename) or SignTool + dlib, with Azure OIDC | SignTool + token on a self-hosted runner, or the CA's cloud signing (eSigner, SimplySign) |
| Approval per release | **Required and enforced** (a maintainer approves each signing request in SignPath) | None (automatic) | None |
| Timestamping | Yes (SignPath) | Yes (Microsoft RFC 3161 service) | Add one yourself |
| SmartScreen | Reputation builds over time; shared publisher identity (see below) | Reputation builds over time; Microsoft: "does not provide instant SmartScreen trust" | The same; EV gives no head start |
| Lock-in | Moving to your own certificate later resets publisher reputation (new identity) | Cannot be moved to another subscription or tenant | Re-key at each reissue |
| Signs MSI | Yes (`<msi-file>`) | Yes (all SignTool formats) | Yes |

Microsoft Store (MSIX) re-signs and has no SmartScreen warnings at all, and is free; it does not fit Kora today (an
installer that creates services, firewall rules and scheduled tasks, and a self-updater) and is not covered further.

### SignPath Foundation: conditions (terms page, 2026-11-17)

OSI licence without commercial dual licensing; no malware/PUA; no proprietary components; actively maintained;
**already released in the form to be signed**; documented on its download page; you sign only your own projects and
binaries; **no hacking tools** (nothing that identifies or exploits vulnerabilities); users' privacy respected,
system changes announced, uninstall provided; all team members use **MFA on GitHub and SignPath**; roles authors /
reviewers / approvers assigned; a **"Code signing policy"** section on the home page and download page containing the
sentence "Free code signing provided by SignPath.io, certificate by SignPath Foundation", the team roles and the
privacy policy (or the "will not transfer any information" sentence); **file metadata restrictions** on every signed
binary (product name = the project's name, product version consistent); builds from source in a verifiable way
(GitHub-hosted runners for OSS); **every release manually approved**. They may refuse or later revoke, with no
arbitration; "a certain verifiable reputation" is required for executables. Kora's `signpath-application.md` (the
owner's draft) already maps its state to these conditions.

Kora against them, from this repository: Apache-2.0, public, released (v0.7.0), builds on GitHub-hosted runners,
announces and lists system changes and has `kora uninstall`. Gaps to close before applying are in the owner steps.

## Recommendation

1. **Apply to SignPath Foundation now** (free, the application is already drafted, no geographic or legal-entity
   restriction in the terms, the manual approval per release matches Kora's draft-release flow). Everything in this
   repository is ready for it and is a no-op until the secret and variables exist.
2. **If SignPath declines or is too slow**, use **Azure Artifact Signing** *if* a registered organization (or a US/
   Canadian individual) exists to be validated: about $120/year, no hardware, fully automatic (no approval
   step). `sign-windows.yml` would get a second backend; the verification, checksum and release plumbing stays.
3. **A paid OV certificate last**: it needs a token or cloud-signing subscription and a yearly re-issue for about the
   same SmartScreen result. **Do not pay for EV** for SmartScreen: it no longer helps.
4. In every case: sign **every** release with the **same publisher identity** (reputation accrues to it), keep the
   file hash stable once released (never re-sign or re-timestamp a published file), and tell early users they may still
   see a prompt for the first weeks (README already says so).

One thing is better with SignPath than with a per-project certificate: the signer is "SignPath Foundation", shared by
many projects, so publisher reputation does not start from zero for Kora. I could not verify how much that helps;
treat it as plausible, not promised.

## What the owner must do or provide

Nothing here can be done by an agent: accounts, MFA, a person approving.

1. **Enforce 2FA** for the KoraFiveM organization (Settings → Authentication security) and on every account that will
   have a role in SignPath.
2. **Settle the product name.** SignPath requires every signed binary's *product name* to equal the project's name and
   enforces it with the artifact configuration. The exe's `ProductName` is `Kora` (`crates/kora-cli/build.rs`), the
   draft application says the project is "Kora Installer". Pick one: register the SignPath project as **Kora** (no code
   change), or change `ProductName` and `.signpath/artifact-configuration.xml` together. (Also in `build.rs`:
   `CompanyName` is "Kora" with a TODO.)
3. **Update the application draft** (`/home/kora_work/signpath-application.md` is yours, I did not touch it): it still
   says v0.1.0/v0.2.0; the project is released as v0.7.0. Its "Code signing policy URL" points at CODE_SIGNING.md, so
   that file needs the section below.
4. **Add the "Code signing policy" section** (below) to `README.md` (home page), `CODE_SIGNING.md` and the releases
   page text, with named maintainers: SignPath asks for individual handles for approvers.
5. **Submit the application** at https://signpath.org/apply. Wait for approval (no fixed time published).
6. In SignPath (after approval): create the project (slug e.g. `kora-installer`), add the predefined **GitHub.com**
   trusted build system and link it to the project; create the artifact configuration from
   `.signpath/artifact-configuration.xml`; create the signing policy `release-signing` (certificate: the Foundation's
   OSS certificate, **manual approval on**, your approver users, optional pipeline policy from
   `.signpath/policies/release-signing.yml` at level "Log" at first); create a **CI user** with submitter rights and
   copy its **API token**; note the **organization ID**. Install the SignPath GitHub App if SignPath asks for audit-log
   evaluation.
7. In GitHub (repository or organization settings → Secrets and variables → Actions):
   * secret `SIGNPATH_API_TOKEN`
   * variables `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG`, and optionally
     `SIGNPATH_ARTIFACT_CONFIG_SLUG` (if not the default).
   * If you keep the pipeline policy's `github-scm-policies`, an **active branch ruleset** on `main` (no force push, no
     deletion) must exist, or SignPath rejects the request.
8. Merge the `release.yml` change below (it belongs to the installer workstream's file), cut a **release candidate tag**
   (e.g. `v0.8.0-rc1`), approve the signing request in SignPath, and check the result (next section).
9. Switch `require-signing: true` in the `sign` job once it has worked, so a broken setup can never ship an unsigned
   release unnoticed. Switch the pipeline policy to "Enforce".
10. Change the README, CODE_SIGNING.md and `SECURITY.md` ("The unsigned Windows executable ... SmartScreen") wording from
    "not signed" to "signed by SignPath Foundation; SmartScreen may still warn for a few weeks".
11. For each release: after the tag build reaches the SignPath step, **approve the signing request** (SignPath emails
    the approvers). The workflow waits up to 4 hours (`approval-timeout-seconds`); re-run the failed job (not just
    re-trigger) if it timed out.

### The "Code signing policy" text to add (README / CODE_SIGNING.md / download page)

```markdown
## Code signing policy

Free code signing provided by [SignPath.io](https://signpath.io), certificate by [SignPath Foundation](https://signpath.org).

- Authors and reviewers: <link to the KoraFiveM members team / list of GitHub handles>
- Approvers (every release is approved by hand): <link to the owners / handles>
- Only binaries built from this repository by [`release.yml`](.github/workflows/release.yml) on GitHub-hosted runners
  are signed. Kora signs nothing else.
- Privacy: Kora contacts nothing unless you ask it to (see the privacy section of the README).
```

## The files in this repository

| File | What it is |
|---|---|
| `.github/workflows/sign-windows.yml` | Reusable workflow (`workflow_call`). Downloads an unsigned artifact, checks `ProductName`, uploads the files for SignPath, submits the request, **verifies** every signature (`Get-AuthenticodeSignature` Valid, subject contains the expected publisher, timestamp present, `signtool verify /pa`, the signed exe still runs `--version`), replaces the artifact **under the same name** with the signed files and **regenerated `.sha256` files**. No-op with a notice when the secret/variables are missing (fails instead with `require-signing: true`). Never ships unsigned once configured: any rejection, timeout or failed check fails the job. |
| `.signpath/artifact-configuration.xml` | The artifact configuration to paste into SignPath: a ZIP with `kora*.exe` and `*.msi` (both optional), `authenticode-sign`, product-name/version metadata restrictions. |
| `.signpath/policies/release-signing.yml` | Optional pipeline policy (GitHub-hosted runners only, branch rules). |
| `docs/release-signing.md` | This file. |

## `release.yml` change (the installer workstream owns that file; this is the exact snippet)

Add a job between `build` and `verify`, and make `verify` wait for it:

```yaml
  sign:
    needs: build
    # Only real releases: a rehearsal would otherwise wait for a maintainer to approve a signing request.
    if: startsWith(github.ref, 'refs/tags/v') && (github.event_name == 'push' || inputs.publish)
    permissions:
      contents: read
      actions: read
    uses: ./.github/workflows/sign-windows.yml
    with:
      unsigned-artifact: kora-windows-x86_64.exe
      # require-signing: true   # switch on once the first signed release worked
    secrets: inherit            # SIGNPATH_API_TOKEN

  verify:
    needs: [build, sign]
    # A skipped `sign` (rehearsal) must not skip verify.
    if: ${{ !cancelled() && needs.build.result == 'success' && (needs.sign.result == 'success' || needs.sign.result == 'skipped') }}
    ...
      - uses: actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2
        with:
          path: dist
          pattern: kora-*          # NEW: never `signpath-input-*` (the unsigned copy SignPath reads)
          merge-multiple: true
```

and, in `verify`, after "Every expected asset exists and checksums verify" (the existing `sha256sum -c` already proves
the `.sha256` file describes the signed bytes, because `sign-windows.yml` rewrote it):

```yaml
      - name: The Windows exe carries a signature when signing ran
        if: needs.sign.result == 'success'
        run: |
          python3 - dist/kora-windows-x86_64.exe <<'PY'
          import struct, sys
          d = open(sys.argv[1], 'rb').read()
          pe = struct.unpack_from('<I', d, 0x3c)[0]
          assert d[pe:pe+4] == b'PE\0\0'
          opt = pe + 24
          magic = struct.unpack_from('<H', d, opt)[0]
          dd = opt + (112 if magic == 0x20b else 96)          # data directories
          off, size = struct.unpack_from('<II', d, dd + 4 * 8)  # entry 4: the certificate table
          assert off and size, 'no Authenticode signature in the Windows exe'
          print('certificate table:', off, size)
          PY
```

Also update the header comment ("no Authenticode signature yet") and keep `attest` as it is: it downloads
`release-assets`, which `verify` assembles from the signed artifacts, so **the attestation is of the signed file**
(it must be: attest after sign, never before). Order of every release job: build → **sign** → verify (checksums) →
attest → publish.

**The MSI (installer workstream):** the MSI embeds `kora.exe`, so build it from the *signed* exe: its job needs `sign`
and downloads the signed `kora-windows-x86_64.exe` artifact, builds `kora-windows-x86_64.msi`, uploads it as an
artifact named `kora-*` with its `.sha256`, then a second `sign-windows.yml` call
(`unsigned-artifact: kora-windows-x86_64.msi`) signs the MSI and rewrites its checksum. Fill in the MSI's `subject` /
`author` restriction in `.signpath/artifact-configuration.xml` (the TODO there) to what the WiX build sets. Add the MSI
to the `attest` `subject-path` and the expected-assets list.

## Does a signed exe break self-update, `install.ps1` or the service's pinned copy?

I read the code (not run on Windows):

* **Self-update** (`crates/kora-core/src/self_update/mod.rs: apply`) downloads the asset, hashes **the whole file as
  downloaded** and compares it to the release's `.sha256`; it checks nothing about Authenticode. It stores the same
  hash as the app's `sha256` (`Applied.sha256`). A signed file is just a different, slightly larger file (the
  signature is appended to the PE; the limit is 100 MB). The only requirement: the `.sha256` must be of the signed
  file. `sign-windows.yml` regenerates it from the signed bytes and re-checks every checksum file before uploading.
  The format is the one `parse_checksum` accepts (`<hex>  <file name>`, lower case, LF).
* **`install.ps1`** does the same with `Get-FileHash` on the downloaded file. Unchanged.
* **The service's pinned copy** (`helper/winhelper.rs`): the installer copies the running exe to
  `%ProgramFiles%\Kora\helper\kora.exe` and checks the **copy's SHA-256 against the source**, and its self-refresh
  downloads "that release ... checks it against the release's SHA-256 (the self-update path)". Both compare the bytes
  they have with bytes they were given, so a signed exe works as long as every hash is taken after signing. The
  rules that keep it true: sign **before** computing any `.sha256` and before the attestation; never re-sign or add a
  timestamp to a published file (the hash would change and the release's `.sha256` would go stale); an MSI must carry
  the signed exe. Moving from an unsigned release to a signed one is an ordinary version update for all of these (new
  file, new hash, same flow). Windows does not re-check the signature when the service starts its own copy.
* **Optional hardening for later (not done):** the self-updater and the service could also call `WinVerifyTrust` and
  require the SignPath Foundation (or Azure) publisher on the downloaded file in addition to the checksum, so a
  tampered release page (file *and* checksum replaced) would still be refused. It needs a publisher pin that survives
  a change of signing provider and must not lock out users on unsigned old releases; it is a design decision for the
  service workstream, and is cheap to add once signing is live.

## Verifying a download (for users and for the README)

Windows:

```powershell
Get-AuthenticodeSignature .\kora-windows-x86_64.exe | Format-List Status, StatusMessage, SignerCertificate, TimeStamperCertificate
signtool verify /pa /v .\kora-windows-x86_64.exe      # needs the Windows SDK; Get-AuthenticodeSignature does not
(Get-FileHash .\kora-windows-x86_64.exe -Algorithm SHA256).Hash.ToLower()   # compare with the .sha256 file
gh attestation verify kora-windows-x86_64.exe --repo KoraFiveM/kora-installer
```

`Status` must be `Valid` and the signer's subject must name the publisher (SignPath Foundation).

## Linux trust

Today: every file has a `.sha256` and a signed **build provenance attestation** (GitHub's Sigstore-based keyless
signing via `actions/attest-build-provenance`, job `attest` in `release.yml`), which proves the file was built by this
repository's release workflow from a given commit and tag, and was not uploaded by hand. `install.sh` verifies the
SHA-256 before running anything. A user checks a download with:

```sh
sha256sum -c kora-linux-x86_64.sha256
gh attestation verify kora-linux-x86_64 --repo KoraFiveM/kora-installer   # prints workflow, commit and tag; fails otherwise
```

`gh attestation verify` can also run offline against a downloaded bundle (`--bundle`) and can pin the workflow
(`--signer-workflow KoraFiveM/kora-installer/.github/workflows/release.yml`) for a stricter check.

**The planned `.deb`** (recommendation only):

* **Do not sign the `.deb` file itself** (`debsigs`/`dpkg-sig`). `dpkg` ignores per-package signatures on Debian and
  Ubuntu by default (`no-debsig` is set in `/etc/dpkg/dpkg.cfg`); SignPath's reference documents the same, and lists `debsigs`
  signing only for its Semantic Code Signing plan (not for the open-source one), so it would buy nothing.
* **Release the `.deb` with the same trust as the binaries**: a `.sha256` next to it, the build provenance attestation
  (add it to `attest`'s `subject-path`), and the install instruction `gh attestation verify` + `sudo apt install
  ./kora_<version>_amd64.deb`.
* **Serve it from an apt repository only if the product wants updates through `apt`.** That is the one real way to get
  signed `.deb` trust (the repository's `InRelease`/`Release.gpg`, verified by `apt` with a key in
  `/etc/apt/keyrings`). Options: a repository on GitHub Pages built with `aptly` or `reprepro` in the release workflow,
  signed with a dedicated GPG key kept as an Actions secret (or in SignPath, whose OSS plan lists GPG-signing for RPM
  and detached GPG signatures; I did not verify that it can sign apt repository metadata), or a hosted service
  (Cloudsmith, packagecloud). Cost: a key to protect and rotate, one more thing to keep alive. **Decision for the
  service workstream:** Kora self-updates its own binary, which for a `dpkg`-owned file fights the package manager
  (`dpkg --verify` drift, `apt` overwriting). If the `.deb` installs the privileged service, let **apt own updates**
  (so the apt repository is worth it) and make the updater tell the person to run `apt upgrade` instead of replacing
  `/usr/bin/kora`; otherwise ship the `.deb` as a one-off installer and keep self-update.

## Unverified (could not check from here)

* How many weeks and downloads it takes for SmartScreen to stop warning for a SignPath Foundation-signed Kora; whether
  sharing the Foundation's publisher identity gives a head start. (Microsoft gives no threshold.)
* The exact subject of the SignPath Foundation certificate (the workflow checks `*SignPath Foundation*`, which the
  terms page implies; if verification fails on the first release the subject printed in the log is authoritative: set
  the `expected-publisher` input).
* That SignPath reads PE `ProductVersion` exactly as the string `crates/kora-cli/build.rs` writes (the workflow reads
  the same field with PowerShell and passes it as the `version` parameter, so a mismatch would show as a clear
  restriction failure on the first request).
* SignPath's waiting time for the application; whether the Foundation will accept the project ("verifiable reputation"
  is required for executables, Kora's is young).
* Azure: eligibility of the owner (organization/individual and country), validation time; the Azure GitHub action's
  inputs (not needed unless SignPath is declined).
* Behaviour of the workflow on a real run: written and YAML/PowerShell-parsed here, never executed. First things to
  watch: the artifact configuration's `min-matches="0"` on a ZIP root, the `version` user parameter, the
  `overwrite: true` replacement of the build job's artifact, and `signtool` being present on `windows-latest`.

## Sources read (2026-11-17)

* SignPath Foundation conditions: https://signpath.org/terms.html
* SignPath GitHub integration and action inputs: https://docs.signpath.io/trusted-build-systems/github,
  https://github.com/SignPath/github-action-submit-signing-request (`action.yml` at `f6d04783`, tag `v3.0`)
* SignPath artifact configuration reference, syntax, examples; pipeline policies:
  https://docs.signpath.io/artifact-configuration/reference, `.../syntax`, `.../examples`, https://docs.signpath.io/pipeline-policies
* Azure Artifact Signing: https://learn.microsoft.com/azure/artifact-signing/overview, `/faq`, `/quickstart`;
  https://azure.microsoft.com/products/artifact-signing (prices); https://learn.microsoft.com/windows/apps/package-and-deploy/code-signing-options
* SmartScreen: https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation
* CA/Browser Forum Code Signing Baseline Requirements §6.3.2 (460 days from 2026-03-01), ballot CSC-31; the hardware
  key rule since 2023-06-01 as restated by CA and reseller pages (DigiCert alert ALERT70, Sectigo, SSL.com, My-SSL).
  Reseller prices are the sellers' own list prices, not quotes.
