## Kora Installer (next release)

### What is new
- **"Production" and "Development" are gone.** A server now has two plain choices. **Where the resources come from:** plain copies of Kora's resources (no GitHub account needed), or your own copies on GitHub that you can change (in your account or an organisation you belong to). **Which versions:** released versions (stable, you choose when to move to a newer one) or the latest changes. The install asks both, and "Which versions" can be changed later on the Resources tab. Nothing moves when you change it: the next update check shows what would.
- **A page for every resource.** Click a resource's name. On its page: check for an update and update just that resource; start, stop and restart it on the running server; its pull requests (with their checks and reviews), your own changes line by line, commit and push, open a pull request, issues, releases, and "Make my own copy". After anything that changes a resource's files the page says **"Restart needed: the running server still has the old version"** until it has really been restarted, for everyone who looks, even after a reload.
- **You can always see what is not on its normal version.** A resource on a branch or a pull request is marked with one badge ("Trying pull request #12: Fix the HUD", "On branch fix-hud") on the resource's page, the Resources list, the servers list, the Overview, the GitHub tab, the update check and the snapshot list, and in `kora status`.
- **Branches, pull requests and the automatic update are for every server.** On a server that follows released versions, trying a pull request is a deliberate choice with a clear warning and one click back to the exact release. Review, comment, approve and merge pull requests from the panel if your GitHub account has the rights. Everything done on GitHub is done as the GitHub account the computer is signed in as, and the panel says which one every time.
- **Command line:** `kora install --source clone|fork --versions released|latest --fork-owner <login-or-org>`; `kora resource ...` (show, changes, log, commit, push, new-branch, issues, releases, fork, versions) and `kora pr view|comment|review|merge`.

- **Run a server in a container (Linux).** When Podman or Docker works, the install can run the server in a container instead of as a system service: nothing is installed on the system, rootless Podman needs no administrator rights at all, and the database is a MariaDB container next to the server. Everything else in the panel works the same (start and stop, logs and console, txAdmin, updates, resources, backups, the reachability check). The System check has a new **Containers** line that says whether it will work and how to fix it when it will not. `kora install --run-mode container`. Backups remember how a server ran, and bringing a server in from a backup offers a container where there is an engine. Docker needs a word of warning, and Kora gives it: being allowed to use Docker is the same as being root, and Docker opens ports around your firewall.

### Upgrading from 0.7.x
- **Your servers are not changed and keep working.** A server that was "production" now reads as plain copies at released versions; one that was "development" reads as your own copies at the latest changes. Both the old and the new words are written to the server's settings, so an older Kora can still read them. `kora install --mode prod|dev` and `--github-owner` still work.
- The **Schedule** and **GitHub** tabs now show on every server. The scheduled update is off until you turn it on; on a server that follows released versions it moves between releases and leaves a resource alone that you put on a branch or pull request.
- The first step of the install wizard asks a different question (see above). Saved answers and stopped installs from the old wizard are read as before.
- Container mode changes nothing for existing servers: they stay system services. A container server has a new file `kora-container.json` in its folder (no secrets). Before going back to an older Kora, make a backup and delete the container server (say so if the database volume should be kept): an older Kora does not know about the containers.
- Going back to an older Kora: nothing to do for these settings (`kora store export-json` writes the old words too). A new file, `kora-resource-state.json`, appears in a server's folder; an older Kora ignores it.

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
