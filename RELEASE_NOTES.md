## Kora Installer v0.6.2

Install and manage a Kora FiveM server from your browser: run `kora panel`.

### Install
Linux:
```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/latest/download/install.sh | sh
```
Windows (PowerShell):
```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/latest/download/install.ps1 | iex
```
Already have Kora? Run `kora self-update`.

### New in 0.6.2
- **Start, stop and restart a single resource** from the server's Resources tab, without typing console commands.
- **The Resources tab remembers what it was doing.** Check for updates, open another tab, come back: the check is still there, and so is what it found.
- **`kora panel restart`** restarts the background panel.
- **`kora panel remote-actions on`** lets you install and delete servers when you reach the panel from another computer (for example over Tailscale). The panel now tells you this before you fill in the install wizard, not after.
- **An easier first install.** The database is set up for you by default, the licence key step links straight to the Cfx.re portal, and a server that only has the root account is told how to create a user for Kora.
- **Server status updates by itself again** in the panel, and buttons you may not use are switched off with the reason instead of failing after the click.

### New in 0.6.0 and 0.6.1
- **Kora Panel no longer runs as root** (0.6.1); `git` run as root trusts less of a repository's own settings.
- **Open Kora Panel from your own devices with Tailscale, in one step.** On the server run `kora panel --tailscale` (the install line offers it when Tailscale is running). Open the address it prints, for example `http://myserver.tail1234.ts.net:47120`, on any device signed in to Tailscale: you are signed in with your Tailscale account, with no link, token or SSH tunnel. Let others in with `kora panel tailscale allow name@example.com --role operator`.
- **Sign-in links work once.** The link `kora panel` prints holds a one-time code, so the panel's token never lands in your browser history.
- **A large security update.** Root no longer trusts files in a server folder (update jobs, delete, uninstall), the panel's session can no longer be taken over through other services on the same machine, proxies and public addresses are refused, and downloads stay on https. Release files now have build provenance: `gh attestation verify kora-linux-x86_64 --repo KoraFiveM/kora-installer`.
- **Restart now** after updating Kora from the panel: no need to SSH in and restart it yourself.
- The install line installs the newest **stable** release (`KORA_PRERELEASE=1` for pre-releases).

### New in 0.5.1
- **The resource list comes straight from GitHub** (the catalog file in the docs repository), so changes to it reach the installer as soon as they are merged. `sd-lib` is no longer offered.

### New in 0.5.0
- **Installing opens Kora Panel for you.** The install line above now sets up Kora Panel in the background (it comes back after a restart) and opens it in your browser. On a server with no screen it prints the `ssh -L` command to run on your own computer. Set `KORA_NO_LAUNCH=1` (Windows: `-NoLaunch`) to only install the program.
- **Paid assets from the Cfx.re Portal.** Download your assets at [portal.cfx.re](https://portal.cfx.re/assets/granted-assets) and install the `.zip` from the panel's new **Paid assets** tab (or `kora add-asset <server> <zip>`). Kora checks the zip, installs it into `resources/[paid]` without renaming anything, adds it to `server.cfg`, keeps backups when you replace it, and explains licence errors from the server console. Buy and download with the same Cfx.re account that owns the server's licence key.
- **Development servers reuse your forks.** Kora no longer forks again (and fails after a five-minute wait) when your fork has a different name, such as `name-1`.
- **Linux: servers go in your home folder.** The suggested folder is `~/kora/<name>`, also under `sudo`. Logged in as root, Kora uses `/home/kora/<name>` with a normal `kora` account.
- **Safer updates.** One update at a time per server, your own commits are never thrown away, nothing is restarted against your restart choice, rollbacks don't throw away changed files, automatic updates can be rolled back, and a version that was rolled back is not retried every night.
- The panel's install wizard shows what it is waiting for instead of looking stuck.

### New in 0.4.2
- **`kora panel` keeps running on a server.** On a computer with no screen (for example over SSH), `kora panel` now offers to run in the background, so it survives closing your terminal. Press Enter, then open the link it prints through the `ssh -L` tunnel. Run `kora panel` again to see the link. `kora panel --foreground` runs it in the current window only.

### New in 0.4.1
- **Sign in to txAdmin with your Cfx.re account.** The install wizards (terminal and panel) now ask for your Cfx.re username, look the account up on forum.cfx.re (only the name is sent) and make it the txAdmin admin, with admin rights in `permissions.cfg`. Leave it empty to keep the generated local admin. From the command line: `kora install --cfx-user <NAME>`.
- **Check one resource at a time.** Each resource on the Updates tab has its own Check button.
- **Much faster update checks.** Resources are checked in parallel, and development servers skip repositories that have not changed (checked with one request to the KoraFiveM organisation on GitHub). Anything uncertain is still fetched.
- Clearer messages when an action is not possible for a server that was added from an existing folder.

The 0.4.0 notes (Kora Panel, helper, monitoring, remote access, hub) are in the [changelog](https://github.com/KoraFiveM/kora-installer/blob/main/CHANGELOG.md).

### Upgrading
Run `kora self-update`, or use the install line above. Servers installed with an earlier version are not changed (they keep their folder); to add your Cfx.re account to a server you already installed, sign in to txAdmin with the local admin from the server's `secrets/txadmin-admin.txt` and add yourself under Admin Manager.
