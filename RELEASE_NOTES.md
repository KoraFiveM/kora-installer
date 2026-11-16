## Kora Installer v0.4.3

Install and manage a Kora FiveM server from your browser: run `kora panel`.

### Install
Linux:
```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/download/v0.4.3/install.sh | sh
```
Windows (PowerShell):
```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/download/v0.4.3/install.ps1 | iex
```
Already have Kora? Run `kora self-update`.

### New in 0.4.3
- **The install wizard no longer looks stuck.** Loading your GitHub accounts, clicking "Install the server" and waiting for the first step all show a spinner and a short explanation now.

### New in 0.4.2
- **`kora panel` keeps running on a server.** On a computer with no screen (for example over SSH), `kora panel` now offers to run in the background, so it survives closing your terminal. Press Enter, then open the link it prints through the `ssh -L` tunnel. Run `kora panel` again to see the link. `kora panel --foreground` runs it in the current window only.

### New in 0.4.1
- **Sign in to txAdmin with your Cfx.re account.** The install wizards (terminal and panel) now ask for your Cfx.re username, look the account up on forum.cfx.re (only the name is sent) and make it the txAdmin admin, with admin rights in `permissions.cfg`. Leave it empty to keep the generated local admin. From the command line: `kora install --cfx-user <NAME>`.
- **Check one resource at a time.** Each resource on the Updates tab has its own Check button.
- **Much faster update checks.** Resources are checked in parallel, and development servers skip repositories that have not changed (checked with one request to the KoraFiveM organisation on GitHub). Anything uncertain is still fetched.
- Clearer messages when an action is not possible for a server that was added from an existing folder.

The 0.4.0 notes (Kora Panel, helper, monitoring, remote access, hub) are in the [changelog](https://github.com/KoraFiveM/kora-installer/blob/main/CHANGELOG.md).

### Upgrading
Run `kora self-update`, or use the install line above. Servers installed with an earlier version are not changed; to add your Cfx.re account to a server you already installed, sign in to txAdmin with the local admin from the server's `secrets/txadmin-admin.txt` and add yourself under Admin Manager.
