## Kora Installer v0.4.1

Install and manage a Kora FiveM server from your browser: run `kora panel`. 0.4.1 adds your Cfx.re account as txAdmin admin and much faster update checks.

### Install
Linux:
```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/download/v0.4.1/install.sh | sh
```
Windows (PowerShell):
```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/download/v0.4.1/install.ps1 | iex
```
Already have Kora? Run `kora self-update`.

### New in 0.4.1
- **Sign in to txAdmin with your Cfx.re account.** The install wizards (terminal and panel) now ask for your Cfx.re username, look the account up on forum.cfx.re (only the name is sent) and make it the txAdmin admin, with admin rights in `permissions.cfg`. Leave it empty to keep the generated local admin. From the command line: `kora install --cfx-user <NAME>`.
- **Check one resource at a time.** Each resource on the Updates tab has its own Check button.
- **Much faster update checks.** Resources are checked in parallel, and development servers skip repositories that have not changed (checked with one request to the KoraFiveM organisation on GitHub). Anything uncertain is still fetched.
- Clearer messages when an action is not possible for a server that was added from an existing folder.

The 0.4.0 notes (Kora Panel, helper, monitoring, remote access, hub) are in the [changelog](https://github.com/KoraFiveM/kora-installer/blob/main/CHANGELOG.md).

### Upgrading
Run `kora self-update`, or use the install line above. Servers installed with an earlier version are not changed; to add your Cfx.re account to a server you already installed, sign in to txAdmin with the local admin from the server's `secrets/txadmin-admin.txt` and add yourself under Admin Manager.
