## Kora Installer v0.8.3

Fixes on top of 0.8.2, from the first testers.

### What is new
- **Schedule no longer says "Off" when it is on.** If Kora cannot see the scheduled job from the panel, it now says so with a warning instead of showing the schedule as off.
- **Backup settings works** from the Backups tab: it takes you to the settings card.
- **Contribute to Kora** in Settings: the three steps to open a pull request.

### Install and update
Run `kora self-update`, or use the install lines from the 0.8.0 notes below (MSI, `install.ps1`, .deb or `install.sh`). Your servers keep working.

## Kora Installer v0.8.2

Fixes and small additions on top of 0.8.1.

### What is new
- **Force restart after an update.** When the panel runs in a terminal window it cannot restart itself, so the update used to end with "stop it and run `kora panel` again". There is now a **Force restart** button: the new version starts in the background (no window) and the page reconnects.
- **Browse... for the server folder** in the install wizard. It opens the normal folder window, in the desktop app or when the browser is on the same computer as Kora.
- **"Players cannot join my server"**: a step-by-step troubleshooting page (server, firewall, router port forwarding, internet providers that share one address, test from outside), linked from the "Can players reach this server?" card.
- **Uninstall now cleans up the per-user install.** `kora uninstall` (and the panel's Uninstall) also removes the start-up entry, `kora.exe` and its PATH entry, and the Windows Firewall rules for `kora.exe` (needs Administrator; otherwise Kora prints the command to run). Your servers and Kora's own records are kept unless you ask to delete them.

### Install and update
Run `kora self-update`, or use the install lines from the 0.8.0 notes below (MSI, `install.ps1`, .deb or `install.sh`). Your servers keep working.

## Kora Installer v0.8.1

A small Windows fix on top of 0.8.0.

### What is new
- **No more antivirus warning about the panel's start-up file (Windows).** Kora used to start its background panel from a small script file (`Kora Panel.vbs`) in your Startup folder, and some antivirus products reported that file as a trojan. The panel now starts from a normal per-user start-up entry that runs `kora.exe` itself, with no console window and no script. The old file is removed the next time the service is set up, and by `kora panel --remove-service`.

### Install and update
Run `kora self-update`, or use the install lines from the 0.8.0 notes below (MSI, `install.ps1`, .deb or `install.sh`). Nothing else changes; your servers keep working.

If your antivirus already quarantined `Kora Panel.vbs`, you can ignore that: updating replaces it.

## Kora Installer v0.8.0

Install and manage a Kora FiveM server from your browser. Run `kora` (or open the Kora shortcut) and the panel opens.

### Install
**Windows:** download `kora-windows-x86_64.msi` from this page and run it. This is the recommended way: Windows asks for an administrator's approval once, and after that Kora never asks again. Or run the one-line installer in PowerShell, which offers the MSI first:
```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/latest/download/install.ps1 | iex
```
The per-user install still works without administrator rights (answer "n", or use `-PerUser`); the panel then offers to add the administrator service later.

**Linux:** download `kora-linux-x86_64.deb` (Debian and Ubuntu) and install it with `sudo apt install ./kora-linux-x86_64.deb`, or use the one-liner, which offers the same:
```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/latest/download/install.sh | sh
```
Already have Kora? Run `kora self-update`, or use the install line again.

### What is new
- **One administrator approval, then no more prompts.** A small Kora administrator service (installed by the MSI or the .deb) does the few things that need administrator rights: services, firewall rules, databases and updating Kora. The permission prompt on every install, and the optional Linux helper, are gone.
- **"Production" and "Development" are gone.** A server now has two plain choices: where its resources come from (plain copies, or your own copies on GitHub) and which versions it follows (released or latest).
- **A page for every resource.** Update, start, stop and restart one resource, work with pull requests, changes, issues and releases, and see "Restart needed" until the server really has the new version.
- **Backups, restore and moving a server.** Back up a server (optionally with a password), restore it with one click and an undo, schedule backups, and bring a server to another computer, Windows or Linux.
- **Run a server in a container (Linux).** With Podman or Docker, a server and its database can run in containers instead of as a system service.
- **Safer remote access.** The panel serves HTTPS, and people you invite sign in with a passkey or one-time codes instead of a token link. API tokens are for scripts only.
- **A proper data store and an activity log.** Kora's own data is in one database file, so two things at once can no longer overwrite each other. Settings shows an activity log of what was changed, by whom. Saved licence keys can live in your system keychain.
- **Can players reach my server?** A checklist for the server, firewall, router and internet, with optional automatic port forwarding. Open txAdmin from another computer through the panel.
- **The Kora desktop app (optional).** A tray icon that shows whether your servers are healthy, and a window for the panel, with notifications.
- **Support bundle.** One file you can look through first and send when you need help. Kora never uploads it, and secrets are removed.
- **Languages and accessibility.** The panel can be shown in German (beta), and works better with keyboards and screen readers.

### Upgrading from 0.7.x
Your servers are not changed and keep working.

**Windows**
1. Run the MSI (or the one-line installer). It installs the administrator service and sets your earlier per-user copy of Kora aside (kept with `.previous` added to its name).
2. Other accounts that should use the panel's administrator parts are allowed with `kora service allow <account>` from an administrator window (the account that installed is allowed already).
3. Open Kora from the Start Menu: the first start moves your data into the new store (see below).

**Linux**
1. Install the .deb (or run the one-liner). It removes the old helper; accounts that were allowed stay allowed. Servers set up the old way are recorded as yours.
2. Add other accounts with `sudo kora service allow <account>`.

**Both**
- Kora's data (server list, saved keys, remembered names, stopped installs, remote settings) moves into `store/kora.db`. The old files are kept in `store/before-sqlite/`. `kora store export-json` writes the old files again if you ever go back to 0.7.
- "Production" is now "plain copies, released versions" and "Development" is "own copies, latest changes". `kora install --mode prod|dev` still works.
- People who signed in remotely with a token link are asked to set up a passkey or one-time codes. Their old token still works in a script.
- `kora helper` and `kora admin-worker` are gone. Sizes in the panel are now shown in decimal units (kB, MB, GB).

### Known limitations
- The Windows and desktop downloads are not code-signed yet, so Windows SmartScreen may warn you. Check the SHA-256 file next to each download if you want to be sure.
- German is a machine-assisted beta; wording may be rough in places.
- Container mode is Linux only, and a server cannot yet be switched between a service and a container in place.
- On a very slow connection the MariaDB download used by `--db install` can take a very long time.

Full list of changes: [CHANGELOG.md](https://github.com/KoraFiveM/kora-installer/blob/main/CHANGELOG.md).

## Kora Installer v0.7.0

Install and manage a Kora FiveM server from your browser. Run `kora` (or double-click the Kora shortcut) and the panel opens.

### Install
Linux:
```sh
curl -fsSL https://github.com/KoraFiveM/kora-installer/releases/latest/download/install.sh | sh
```
Windows (PowerShell):
```powershell
irm https://github.com/KoraFiveM/kora-installer/releases/latest/download/install.ps1 | iex
```
Already have Kora? Run `kora self-update`, or use the install line again.

### What is new
- **The panel is now the only way in.** The terminal screens are gone. `kora` makes sure the panel is running and opens it in your browser; with no servers yet it opens straight into the install wizard. Nothing about getting started needs "Run as administrator".
- **Shortcuts that belong to you.** The installer adds a Start Menu and Desktop shortcut on Windows, and a launcher in the applications menu on Linux desktops. They open the panel. No more "Kora app" in Program Files.
- **Errors tell you what to do.** When a step fails, the panel says what happened in plain words and offers buttons: try the step again, skip it (when that is safe), sign in to GitHub, install what is missing. No more "close this window and start again".
- **Saved licence keys.** Keep your keys under names ("Main", "Test server") and pick one when you install. Change a server's key later from its Overview tab. Keys are kept in a private file in Kora's folder (plain text, readable only by you) and the panel only ever shows the last four characters.
- **Kora brings its own Git and GitHub CLI** for Development servers (Windows and Linux), downloaded and checked for you. No winget and no restart of your windows. The new **System check** page shows what is ready and fixes what is not.
- **Branches and pull requests.** On a Development server you can switch a resource to another branch or try a pull request from the GitHub tab, one at a time or several at once. Your own changes are never lost, and updates leave a switched resource alone.
- **Quick buttons on every server:** Open txAdmin, Open folder (Copy path when you are not at that computer) and Copy connect address.
- **Remote access.** Administrators who sign in over Tailscale can now install and manage servers without a switch. For sign-in tokens, installing works by default; deleting servers and similar actions still need `kora panel remote-actions on`.

- **Windows asks for permission once, and the install carries on.** An install from the panel no longer stops at "run this command as administrator". Windows shows its normal permission prompt once when the install starts, for the service, the firewall and the database. If you say no, or nobody is at the computer, you can ask again, carry on without those parts, or see the command to run. The prompt can appear as a flashing shield in the taskbar.

### Upgrading from 0.6.x
Servers you already have are not changed.

**Windows**
1. Run `kora self-update` or the install line above. Kora makes the new shortcuts for you.
2. Open Kora, go to **Settings** and press **Tidy up old shortcuts** (the servers screen shows a notice too). That removes the old "Kora app" copy in Program Files and its administrator shortcuts. It needs administrator rights once, so the panel shows the command to run (`kora uninstall --app --yes`) or you can run it yourself from an administrator window.
3. From now on open Kora from the Start Menu or the Desktop shortcut, not as administrator.
4. `kora install` from the command line now puts new servers in `C:\Kora\<name>` (it used to be `%USERPROFILE%\kora\<name>`), the same as the panel.

**Linux**
1. Run `kora self-update` or the install line above. On a desktop it adds a launcher to your applications menu (skip it with `KORA_NO_SHORTCUTS=1`).
2. If the old machine-wide copy is still in `/opt/kora`, tidy it up from Settings, or run `sudo kora uninstall --app --yes`.

**Things that behave differently**
- `kora manager` and `kora install` with no options now just open the panel. `kora install` with options still installs without asking, for scripts.
- `kora app uninstall` removes only your own shortcuts. The old machine-wide app goes with `kora uninstall --app`.
- The panel's error messages in the API now carry more detail and new, more specific codes. If you script against the panel's API, see the changelog.

Full list of changes: [CHANGELOG.md](https://github.com/KoraFiveM/kora-installer/blob/main/CHANGELOG.md).
