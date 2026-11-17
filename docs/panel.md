# Kora Panel user guide

Kora Panel is the web page that comes with Kora Installer. It is Kora's only interface (since 0.7.0 there is no terminal
manager or wizard): install, manage and monitor FiveM servers from your browser. Everything the command line can do is
also in the panel, with one exception: setup codes no longer exist (see [Setup codes](#setup-codes-are-gone)).

The panel is a program on your own computer, not a website. By default it can only be reached from the computer it runs
on, and nothing you do in it is sent to Kora or anyone else.

Sections 9 to 11 ("Optional") describe features that are added on top of the base panel (monitoring, remote access and
the hub); you can skip them. Everything else describes the base panel.

## Contents

1. [Starting the panel](#1-starting-the-panel)
2. [Signing in](#2-signing-in)
3. [The servers screen](#3-the-servers-screen)
4. [A server's tabs](#4-a-servers-tabs)
5. [Installing a server (the wizard)](#5-installing-a-server-the-wizard)
6. [Settings](#6-settings)
   (and the [System check](#the-system-check))
7. [Actions that need administrator rights](#7-actions-that-need-administrator-rights)
8. [Troubleshooting](#8-troubleshooting)
9. [Optional: monitoring and alerts](#9-optional-monitoring-and-alerts)
10. [Optional: remote access](#10-optional-remote-access)
11. [Optional: the hub (several machines)](#11-optional-the-hub-several-machines)
12. [Security model in short](#12-security-model-in-short)
13. [Setup codes are gone](#setup-codes-are-gone)

## 1. Starting the panel

Run this on the computer that hosts your servers (Windows 10/11 or Server 2019+, or Debian 12+ / Ubuntu 22.04+), or
double-click the Kora shortcut in your Start Menu / on your desktop (a launcher in the applications menu on Linux):

```sh
kora
```

Kora makes sure the panel runs in the background (so monitoring and alerts keep working), then opens your browser at it.
It never asks for administrator rights. With no servers yet the panel opens straight into the install wizard.
`kora install` (with no options) opens the install page. (The old name `kora manager` still works and does the same as `kora`.)

What `kora` does when things are not simple:

| Situation | What happens |
|---|---|
| The panel is already running | It is opened; nothing is started twice. |
| Port 47120 is taken by another program | The next free port up to 47140 is used (the link shows it). |
| No background service is possible (for example a container with no user session) | The panel runs in that window instead; closing the window stops it. |
| A server with no screen (SSH) | Nothing opens. Kora prints the sign-in link, a ready-to-paste `ssh -L` command to run on your own computer first, and the Tailscale option (`kora panel --tailscale`). |
| Typed with `sudo` (Linux) | Kora runs again as the person who typed `sudo` (the panel never runs as root). |
| Typed as root with no `sudo` user (Linux), or in an "administrator" window (Windows) | A running panel is opened (on Windows, if the background panel was set up before, Kora starts it as your normal user). Otherwise Kora explains what to do (log in as a normal user / open Kora from the Start menu or a normal PowerShell window). Nothing is changed. |

For a window that stays open and runs the panel in front, use `kora panel` (below). Kora starts a small web server, prints a line like `Kora Panel: http://127.0.0.1:47120/#code=...` and opens your
browser at that address. The panel keeps running in that terminal; press Ctrl+C to stop it. Closing the terminal also
stops it (servers you started are not affected: they run as services).

Options:

| Option | What it does |
|---|---|
| `--port <PORT>` | The port to listen on. Without it Kora uses 47120, or the next free port up to 47140 when something else uses 47120. |
| `--setup` | Makes sure the background panel is set up and running, opens it in your browser (where there is a screen) and prints the sign-in link. The install scripts run this. It never reads what is piped in; on a server with no screen and Tailscale running it asks once, on the terminal, whether to use Tailscale (`KORA_TAILSCALE=1` or `0` answers in advance, for example in automation). |
| `--no-open` | Do not open a browser; print the address and sign-in link instead. This is automatic over SSH or when the computer has no desktop. |
| `--tailscale` | Open the panel to your own devices over Tailscale (see below). Run once; it keeps running in the background. |
| `--admin <LOGIN>` | With `--tailscale`: the Tailscale account (`name@example.com`) that becomes the panel's admin. Without it Kora uses the account you are connected from, else the one that added the computer to Tailscale. |
| `--bind <ADDRESS>`, `--remote` | Listen on another address than `127.0.0.1` (plain HTTP: only on a private network such as Tailscale or WireGuard); needs remote access (`--remote` for this run, or the saved setting). |
| `--allow-public` | Let `--bind` be a public address or every interface. Not recommended: use `--tailscale` or an SSH tunnel. |
| `--foreground` | Run in this window only; do not offer or use the background panel. |
| `--install-service`, `--remove-service` | Keep the panel running in the background (starts at sign-in, no administrator rights), or stop and remove it. |
| `--print-link` | Print the sign-in link of the background panel. |
| `--new-token` | Make a new sign-in token: links shown before stop working. |
| `--no-elevate` | Do not ask Windows or `sudo` for administrator permission (for developers and CI). |

Run `kora panel --help` to see the options your version supports.

### Starting it on a server with no screen (SSH)

**Using Tailscale? This is the easy way.** Install [Tailscale](https://tailscale.com/download) on the server and
on your own computer or phone (signed in to the same account), then run this once on the server:

```sh
kora panel --tailscale
```

Kora prints an address such as `http://myserver.tail1234.ts.net:47120`. Open it on any of your devices that are
signed in to Tailscale: you are signed in with your Tailscale account straight away, with no link, password or
tunnel. The panel keeps running in the background and comes back after a restart. When the install script finds
Tailscale running on a server, it offers to do this for you. See [Remote access](#10-optional-remote-access)
for who gets in and how to let others in.

**Without Tailscale**, use an SSH tunnel, as below.

On a headless Linux server there is no browser to open, and a panel running in your SSH window would stop when the
window closes. So when you run `kora panel` there, Kora asks:

```
This computer has no screen, so Kora Panel would stop when you close this window.
Keep it running in the background instead? [Y/n]
```

Press Enter to keep it running in the background (a service that starts at sign-in and keeps watching your servers). Kora
prints the sign-in link and the tunnel line:

```
The Kora Panel now runs in the background (starts at sign-in) and keeps watching your servers.
Sign in with: http://127.0.0.1:47120/#code=...
On a server with no screen, open a tunnel from your own computer first:  ssh -L 47120:127.0.0.1:47120 you@203.0.113.7
```

Run `kora panel` again at any time to see the link again (it never starts a second panel while the background one is
running). Answer `n`, or use `kora panel --foreground`, to run it in this window only; `kora panel --no-open` does the
same without asking.

On your own computer, open a second terminal and run the `ssh -L` line it printed (replace `user@host` with your login).
Leave it open, then paste the full link, including the `#code=...` part, into your browser. The tunnel carries the page
over SSH, so nothing is exposed to the network.

### The desktop app (optional)

Instead of a browser tab you can use the **Kora desktop app**: a tray icon (notification area on Windows) with a small
window that shows this same panel. Start `kora-desktop` or the Kora shortcut (`kora app install` points the shortcut at it
when it is installed next to `kora`). The icon is green when every server runs, amber when something deserves a look, red
when something needs attention and grey when the panel is not reachable; its menu opens Kora, starts, stops and restarts a
server, opens txAdmin, checks for updates, pauses notifications and switches "Start with Windows" on and off. Closing the
window keeps it in the tray; **Quit** closes the icon only: your servers and Kora's background panel keep running. It
notifies you when a server stops unexpectedly or an update is ready, using the alert rules of the Monitoring and
Notifications tabs (a rule that is off never notifies). Inside the app, "Add an existing server" has a **Browse...** button
for the folder. The panel itself is unchanged (still `127.0.0.1` only), and the browser route keeps working. Details:
[desktop-app.md](desktop-app.md).

## 2. Signing in

The link that `kora panel` prints contains a one-time code: it works once, within 10 minutes. When you open it:

1. The page sends the code to the panel, which checks it and signs your browser in.
2. The code is removed from the address bar straight away.
3. From then on you stay signed in (also after a reload or in a new tab).

If the link has expired, run `kora panel` (or `kora panel --print-link`) again for a new one. On another computer
you sign in with your Tailscale account (see [Remote access](#10-optional-remote-access)) or a user token.

**Do not share a link** while it is still valid: it signs in as you. Behind every link is the panel's long-lived
token (kept owner-only in `~/.kora/panel-token`), which never appears in a link; replace it with
`kora panel --new-token` if you think it leaked.

### When the session expires

Sessions last until you stop the panel. If you restart `kora panel`, or open the panel in a new browser, you see
"Session expired, or not signed in". To fix it, run `kora panel` again and open the new link. You can also paste the code
(the part after `#code=`) into the box on that page.

Too many wrong attempts at the sign-in box lock it for about a minute.

## 3. The servers screen

"Your servers" lists every server Kora knows about, with its state (running, stopped, starting), whether an update is
waiting, and buttons to start, stop and restart. From here you can:

- Open a server to see its tabs.
- Start the install wizard ("Install a server").
- Add a server that already exists on disk. Kora scans for server folders it can adopt; adopted servers can be removed
  from the list again without deleting any files.

If a server's folder has gone missing, the list says so rather than hiding it.

Each server in the list (and its Overview) has three buttons:

- **Open txAdmin:** opens txAdmin in a new tab. From the computer that runs Kora it goes straight to `localhost` and
  txAdmin's own port. From anywhere else (Tailscale, a LAN address, an SSH tunnel) it goes **through the panel**: the
  panel opens one more port for that server's txAdmin on the same address you reached the panel at (txAdmin's port +
  10000: 40120 becomes 50120), checks your panel sign-in, and passes everything on to txAdmin on the computer. txAdmin's
  own port does not have to be open for that. Details and limits:
  - It is for **administrators** only (txAdmin can run code on the server), and a sign-in made with a token also needs
    `kora panel remote-actions on`, like console input. Signing out of the panel ends it.
  - The panel-side port is only opened where the panel itself listens. Through an SSH tunnel add it to the tunnel
    (`-L 50120:localhost:50120`); with Tailscale and a firewall, allow that TCP port for your devices (for ufw:
    `sudo ufw allow in on tailscale0 to any port 50120 proto tcp`).
  - txAdmin has its own sign-in; the panel cannot sign you in to it (txAdmin has no single sign-on). The button's note
    names the account Kora made at install and where the password is saved.
  - Why not `/txadmin/<server>/` on the panel's own port? txAdmin's web app cannot live under a path: it calls `/auth/...`
    and `/socket.io` from the root and has no base-path setting (checked against txAdmin 8.1.2).
- **Open folder** / **Copy path:** at the computer that runs Kora, with a desktop in front of it, it opens the server's
  folder in the file manager. From anywhere else (Tailscale, a token sign-in, an SSH tunnel, a computer with no desktop)
  it is **Copy path** instead, because opening a window nobody is looking at would do nothing useful.
- **Copy connect address:** this computer's address on your network with the server's game port, for players on the same
  network. For players on the internet use the **Can players reach this server?** card on the Overview (below): it finds
  your public address, checks the firewall and the router, and shows the address and the Cfx.re join link to give out.

### Can players reach this server?

The card on a server's Overview (and on the install wizard's last page; `kora reach <server>` on the command line) is a
checklist with one line per step, each **OK**, **Probably OK**, **Problem** or **Not known**:

1. **Running:** the server answers on its game port over TCP and UDP (FiveM needs both), and on this computer's network
   address, not only on its own loopback address.
2. **Firewall:** this computer's firewall lets the port in. Kora knows what it opened itself; where it cannot see (a
   firewall that needs administrator rights to read, or your hosting provider's), it says so. **Open port ...** opens it
   (needs administrator rights: the card shows the command if Kora cannot ask).
3. **Router:** is there a router in between, is it the kind that cannot forward at all (carrier-grade NAT: the card
   explains and lists the alternatives), and is the port forwarded. If your router answers UPnP, PCP or NAT-PMP, **Forward
   port ... for me** sets it up; otherwise the card lists the exact values to enter on the router's "Port forwarding" page.
4. **Internet:** only a computer outside your network can say. Kora can look your server up on Cfx.re's public server list
   (your server's own join code, read from its log) and, to learn your public address when the router cannot tell it,
   ask a "what is my address" website. **Nothing is contacted unless you allow it**; each lookup has its own switch (at the
   bottom of the card, and `kora reach --cfx-list --remember`), the card says what it contacts and what that learns before
   the first use, and each check lists what it contacted. Being listed is "Probably OK", never proof (only a player joining
   shows the UDP part works), so the line always ends with the manual test: ask a friend who is not on your network to
   press F8 in FiveM and type `connect <your public address>:<port>`.

**Automatic port forwarding** is optional and off by default, per server. A router that programs can reconfigure is a small
risk (any program on your network can do the same), so turning it on asks first. Kora then forwards only that server's game
port (TCP and UDP) to this computer, renews it while the server runs (the server's background service does, or the open
panel for servers started without a service) and removes it when the server stops, when you turn the option off and when
the server is deleted. Turn it off again with `kora reach <server> --forward off`.

## 4. A server's tabs

| Tab | What it is for |
|---|---|
| Overview | State, ports, the update badge, start / stop / restart, the server's folder (Open folder on that computer, Copy path elsewhere), Open txAdmin, the connect address and the **Can players reach this server?** checklist. **Licence key:** which saved key the server uses and **Change licence key** (pick a saved key or add a new one; it says whether a restart is needed and offers it). Same as `kora keys use`. |
| Console | Live server output and a box to send commands. The first time, Kora offers to add a small resource called `kora_console`; it loads on the next server start. |
| Logs | Pick a log file and read its latest lines. |
| Resources | **Updates** at the top: check for newer versions, pick what to update, choose what happens about restarts, and roll back to an earlier snapshot (same as `kora update`). Below it the installed resources, each with buttons to check it for updates and to start, stop or restart it on a running server (needs the live console from the Console tab), and the catalogue of resources you can add (same as `kora add-resources`). What the tab shows stays when you open another tab and come back. |
| Backups | Back up the server (database, settings, txAdmin data, resources), check, restore and undo, delete or download a backup, choose where backups go and how many are kept, schedule them, and **Move this server** to another computer. See [Backups](#backups-and-moving-a-server). Same as `kora backup`. |
| Schedule | When the server updates itself (off until you turn it on). Changing it needs administrator rights. Same as `kora schedule`. |
| Notifications | The Discord webhook for update reports: set, test, remove. The address is never shown back to you. Same as `kora webhook`. |
| Ports | Change the game and txAdmin ports. The server must be stopped. Kora checks the ports are free. Firewall rules are not changed: open the new ports and close the old ones yourself. |
| GitHub | Sign in to GitHub and see where each resource is (each name opens its page); the pull requests, changes, issues and releases are on the resource's page (see below). |
| Danger zone | Delete the server. You choose whether to keep the files and the database, then type the server's name to confirm. |

### Backups and moving a server

**What a backup is.** One archive file (`.tar.gz`, or `.tar.gz.age` when it has a password) and a small listing file beside it. It holds the server's own database (a consistent copy: one snapshot, with routines, triggers and events), `server.cfg`, `permissions.cfg`, the secret files, txAdmin's data, Kora's records for the server, and the resources. A resource is saved **by reference** (where it comes from and the exact version, so the file stays small and is fetched again on restore) unless it cannot be fetched again: changes you made, changes kept aside, commits that exist only on your computer, paid assets and folders that are not git copies are saved **in full**. "Keep every resource in full" saves them all (bigger, needs no internet to restore). The FXServer program is not in a backup (it is downloaded again).

**Secrets.** A backup contains your licence key, the database login and the txAdmin admin: it has to, or it could not restore the server. The file is readable only by your account, only an administrator can download it, and you can lock it with a password (the standard `age` format, so `age -d file.age` opens it). **Kora does not keep the password and nobody can recover it**: a lost password means a backup that cannot be opened. Passwords are off by default for backups kept on this computer (the same secrets already sit in plain text beside them) and **required** for a file made to move a server (it travels). Scheduled backups cannot use a password (nobody is there to type it); put them on a disk only you can read.

**While the server runs.** The database copy is one consistent snapshot. The other files are copied one by one, so two of them can be from slightly different moments (Kora notes a file it saw change). For a strictly consistent copy choose "Stop the server while the backup runs": players are disconnected and the server starts again afterwards.

**Where they go.** Kora's own backups folder (in the Kora folder, outside every server folder, so deleting a server does not delete its backups), or a folder you choose, such as another disk or a mounted share. A backup on the same disk does not survive that disk dying: keep one somewhere else too. Kora checks there is room first. **How many to keep:** the newest N, and optionally one per day for D days; older ones are removed after each backup. A safety copy from a restore is kept 30 days; a copy made for moving is never removed.

**Scheduled backups** run **only while Kora's background service runs** (`kora panel --background`, set up in [section 1](#1-starting-the-panel)): they are made by the panel, not by the operating system's scheduler, which would need administrator rights. A slot missed while it was off is made up once when it starts again. A failed backup raises an alert (and goes to your Discord and webhook), and the servers list and Overview show "Last backup 3 days ago" with a warning when the newest is older than the schedule says.

**Restoring.** Pick a backup, press **Restore…**, read what will change and what stays, and confirm. Kora checks every file against its checksums first (nothing is touched until the file proves good), makes a **safety copy** of the server as it is now, stops the server, puts the database back, then the files, then the resources (fetched back to the saved versions; one that cannot be fetched is listed and left as it is), and starts the server again. `secrets.cfg` and `database.json` of the running server are kept (they hold today's licence key and login). **Undo this restore** puts the safety copy back. A backup made by a newer Kora is refused; a wrong password, a damaged file and a backup of another server are explained.

**Who can do what.** Operators can make a backup (into the usual folder), check one and preview a restore. Administrators can restore, undo, delete, change the settings, choose another folder and download a backup; a remote sign-in made with a token also needs `kora panel remote-actions on` for those. Downloads and restores are in the activity log.

**Moving a server to another computer (Linux or Windows).**
1. On the old computer open the server's **Backups** tab and use **Move this server**: choose a password and make the file.
2. Copy the file to the new computer (USB stick, a share), or download it from the old panel.
3. On the new computer open Kora, go to **Install a new server** and use **Bring a server from a backup or another machine** (or run `kora backup import <file>`). Upload the file (or give its path on that computer), enter the password, and read the preview: what the server is, what will be created, what does not come along, and anything in the way (the name, ports or folder are already used; each comes with a fix).
4. Kora builds the server in steps. If one fails, run the same import again to carry on from where it stopped (or **Start over** to remove what was made).
5. Afterwards: open the ports (the Overview's reachability check helps), set up the background service if you want one, and when the new server works, stop it on the old computer and remove it there (Danger zone). Kora never acts on the other computer for you.

*Clone or fork:* the backup records whether the server follows plain copies of the original repositories or your own copies (forks), which versions it follows, and for every resource whether it is on its normal version, on a branch or trying a pull request. A restore puts every resource back exactly so, and marks them all as changed ("Restart needed" shows for what the running server has not loaded). If the server follows copies in a GitHub account the new computer's sign-in cannot use, the import asks: keep the remotes as recorded, make your own copies here, or use the originals.

*Deleted servers:* when you delete a server, its backup settings go and its backups are kept; the import page lists them under "Backups of servers that no longer exist" so one can be restored as a new server.

*What does not carry over:* the operating-system service, firewall rules and port forwarding, the automatic-update timer and the backup schedule, your GitHub sign-in, and the FXServer program (downloaded again). Folders in the settings and txAdmin's files and line endings are converted for the new system; a resource that runs a program only one system has will not work on the other. The database goes into a **new** database and login on the new computer; the old login is not reused.

*The licence key:* it moves with the server, but it is yours and Cfx.re ties it to your account and the address the server runs on. Kora cannot make Cfx.re accept it on a new computer. If the server says the licence is not valid, check the key at portal.cfx.re (Keymaster) and its server address, or pick another saved key while importing.

### Getting help: the support bundle

**Settings → Get help → Create a support bundle** (also a small link in every error's details, and `kora support bundle [<server>]`) makes one zip to give to whoever is helping you: Kora's version, your system, the System check, tool versions, your servers' settings, recent jobs and problems, the activity log, install and update logs, the service, firewall and disk state. **Every secret is removed** (licence keys, database logins, txAdmin accounts and tokens, webhook addresses, Git and GitHub tokens, passwords, people's identifiers, user names in folder paths) and files that hold secrets (`secrets.cfg`, `database.json`, txAdmin's data) are never read, only listed as left out. You see the list and can read any file before it is saved; **it is never uploaded**, it only leaves your computer if you send it. Not removed: server names, ports, folder names, public IP addresses, resource names and ordinary log text: look through the files you are shown. The GitHub tab's **Report an issue** can add a short cleaned summary (never the file).

### Branches and pull requests (Development servers)
### The page of one resource

Every resource name in the Resources tab opens its own page (`/servers/<server>/resources/<resource>`, so it can be bookmarked and the back link returns to the list). It shows where the resource's files are and has **Check for update**, **Update** (only this resource; it does not restart anything), and **Start**, **Stop**, **Restart** on the running server. The last three send the command through the live console, so they need the server running and the console switched on: when they cannot work the page says why and offers the fix (start the server, or open the Console tab). Checking and the three buttons need the operator role, updating needs admin.

**Restart needed.** After anything that changes a resource's files (update, switching branch, trying a pull request, going back to the normal version, bringing kept-aside changes back, rollback) the page says "Restart needed: the running server still has the old version." with a restart button. Kora remembers this on the server in `kora-resource-state.json` in the server folder, as a change number per resource against the number of its last restart and of the last start of the whole server, so reloading the page, another user or `kora resource show` all see the same thing. It goes away when the resource is restarted from the page or the whole server is started or restarted through Kora. A server that is not running has nothing old loaded, so it shows no notice. A restart outside Kora (for example from txAdmin) clears it too on Linux, because Kora reads when the game server process started; on Windows it does not read that yet, so restart from the page or Kora. Delete the file to clear it.

### The GitHub work of one resource

Everything you do with a resource's code is on its page; the GitHub tab is only the sign-in, the list of where each resource is (each name opens its page) and a pointer. Every server has all of it, whatever it follows: some parts need your own copy of the resource (a fork) and say so, with a **Make my own copy (fork)** button.

- **Which version of the code.** The resource's page header and the Resources list show **where its files are**. On its normal version nothing is shown but a quiet "Normal version (dev)" (or "Released version (abc1234)"); on anything else a warning badge in one colour with words and an icon: "On branch fix-hud", "Trying pull request #12", "On a single commit". The same badge is in the servers list ("2 resources not on their normal version"), the Overview, the GitHub tab, the bulk dialogs, the snapshot list and the update check, and in `kora status`, `kora branch list` and `kora resource show` (lines starting with `!`).
  - Switch to a branch (branches already in the folder, on your own copy, on the original; "Refresh list" asks GitHub), start a **new branch** from where it is, or go **back to the normal version**. For a server on released versions "normal" is exactly the pinned release (on no branch), and **Back to the released version** returns to that commit in one click.
  - Several resources at once: type a branch name, check which have it, switch those.
  - **Your own changes are never lost.** A resource with changes you have not saved is not touched until you choose *Keep my changes aside* (recommended), *Throw my changes away* (asked twice) or *Cancel*. Kept-aside changes are an ordinary git stash starting with "Kora:", listed with *Bring my changes back*. Kora never deletes a stash.
  - **A server on released versions** shows: "This server follows released versions. This resource will run unreleased code until you put it back on its released version", with a box to tick before any switch or pull request. Switching needs the admin role. Updates leave a switched resource alone, with the reason in the update check and the scheduled update's report; going back makes updates work for it again.
  - After any of this the page says "Restart needed" (see above).
- **Pull requests.** The open pull requests of the original repository and of your own copy, labelled, each with title, number, author, age, whether its checks pass, fail or are running, whether it is approved or has changes requested, a link to GitHub and a search box. **Details** shows the checks that are not passing, the files it changes, the conversation (comments and reviews) and, for people allowed to, **Comment**, **Approve**, **Ask for changes** and **Merge** (the merge method the repository allows, never forced past its rules, asked first). **Try it** downloads one onto a branch `pr/<number>`; the one being tried is shown first with how far behind its newest commit the folder is, **Get the latest commits** and **Stop trying it**. The "this is someone else's code that will run on your server" warning is always shown.
- **Your changes.** What changed in the folder (files, and the changes line by line, read-only), **Save the changes (commit)** with a message, **Push** to your own copy (never forced; refused on a plain copy, on a single commit and for a pull request you are only trying), **open a pull request** to the original author, and the recent commits.
- **Issues and releases.** Open issues of the resource's repository with comments and a form to report a new one (as before, with Kora's version and no secrets); the releases, which one the server is pinned to, what is newer and the release notes.
- **Make my own copy (fork).** Turns a plain clone into your own copy for one resource or the whole server: forks it into your account or an organisation, makes `origin` your copy and keeps the original as `upstream`. The version, the branch and any changes you have stay exactly as they are. It needs a GitHub sign-in and the right to create repositories where the copy goes; without them the usual guided message offers the sign-in.

**Whose GitHub account acts.** Everything above that happens *on GitHub* (push, comment, review, merge, issue comment, pull request, fork) is done as the one GitHub account `gh` on the computer that runs Kora is signed in as, shared by every person who uses the panel. The page says "This is done on GitHub as <login>" next to each such button and after each such action. These actions need the admin role, and a remote administrator who signed in with a token is held back from push and merge unless remote actions are switched on (`kora panel remote-actions on`), exactly as for other outward-facing changes. Reading (changes, log, pull request details, issues, releases) needs operator, the log viewer.

The same operations from a terminal: `kora branch ...`, `kora pr list|checkout|view|comment|review|merge ...`, `kora resource show|changes|log|commit|push|new-branch|issues|issue-comment|releases|fork|versions ...`.

## Clone / fork and released / latest (what changed from Production and Development)

Before 0.8 a server was *Production* or *Development*, and that one word decided many things at once. It is now two separate settings, which every server has:

- **Where the resources come from** (`source`): `clone` (plain clones of the original repositories, no GitHub account needed) or `fork` (your own copies on GitHub, in your account or an organisation).
- **Which versions** (`versions`): `released` (each resource pinned to a released version of the catalogue; updates move between releases) or `latest` (each follows the main branch).

Old servers keep working untouched: a record without the new fields reads as `production` = clone + released and `development` = fork + latest. The old `mode` word is still written next to the new fields (`production` when the versions are released, `development` otherwise) so an older Kora can read the file. `kora install --mode prod|dev` and the API's `mode` still work; `--source clone|fork`, `--versions released|latest` and `--fork-owner <login-or-org>` are the new spelling and win over it. With only a source given, the versions default to released for clones and latest for forks.

| What differed | Now |
|---|---|
| Resources are clones of the originals / forks in your account | **source**: `clone` or `fork`. Needs GitHub (and `gh`) only for `fork`. |
| Pinned to catalogue releases / following the fork's default branch | **versions**: `released` or `latest`. A separate setting, changeable later (Resources tab; `kora resource versions`). Nothing moves when it changes: the next check shows what would. Going to released only moves a resource forward to a release that contains what it has; one that is ahead of its release is shown as needing attention, never moved back. |
| Fork sync from the original before an update | follows **source** (`fork`). |
| Which resources can be picked (development-only resources are not released) | follows **versions** (`latest` offers them). |
| Daily automatic update (Development only) | **every server**. Off until turned on; the installer sets it up by default only for `latest` servers with a background service. On a released server it moves to newer releases through the same engine as a manual update; on a latest server it fast-forwards the branches. |
| Branches, pull requests, switching a resource | **every server** (decision: see "The GitHub work of one resource"). The stability warning is about running unreleased code on a server set to released versions, not about forking. |
| Report an issue / open a pull request | every server for issues; a pull request, push and fork sync need your own copy (`fork`) and offer *Make my own copy*. |
| GitHub sign-in asked up front | only when making your own copies. |
| Wording (servers list, Overview, wizard, review page, CLI output, issue diagnostics) | the plain words above: "Plain copies, released versions", "Own copies, latest changes". |
| Ports, licence key handling, notifications, firewall, uninstall | **the same for both** (they never really depended on the mode). |

On disk: `kora-server.json` and `servers.json` gain optional `source` and `versions` fields (see `installer::style` in the code for the single mapping); saved install answers and a stopped install carry them too. Nothing is rewritten until the record is saved for another reason.


## 5. Installing a server (the wizard)

"Install a server" walks through the install steps (`kora install` with options does the same without asking):

1. **Where the resources come from:** *Use Kora's resources as they are* (plain clones, no GitHub account needed) or *Make my own copies on GitHub so I can change them* (forks, then: your personal account, or an organisation you belong to).
   **Which versions:** *Released versions* (recommended, stable) or *Latest changes* (the main branch). It starts as released for clones and latest for your own copies, and can be changed later on the Resources tab.
2. **Name and folder:** the server name, and where it lives. A suggested name and folder are filled in.
3. **Ports:** game port (30120 by default) and txAdmin port (40120). Kora checks they are free and suggests the next free
   ones if they are not.
4. **Build:** the recommended FXServer build, or one you choose (advanced).
5. **Resources:** pick what to install. Dependencies and the core resources are always included.
6. **Database:** use the MariaDB already on the machine, let Kora install it, or skip it. "Test login" checks the admin
   password before anything starts.
7. **Options:** the background service and the firewall rule (the server is started when the install finishes).
8. **Licence key:** pick one of your saved keys by name, or add a new one. A pasted key is saved on this computer by
   default (under a name you can change, "Main" at first) so next time you only pick it; untick "Save this key" to use it
   once. Kora only ever shows the last 4 characters of a key. Roles other than admin can only pick a saved key.
9. **Your Cfx.re account:** type your Cfx.re username and press "Check". Kora asks forum.cfx.re (only the username is sent)
   which account owns it, and that account becomes the txAdmin admin, so you sign in to txAdmin with Cfx.re. Leave it
   empty to skip: Kora then creates a local txAdmin admin with a generated password saved in the server folder. After
   the first time Kora remembers your name (and, when you make your own copies, your GitHub owner) and fills them in; change them
   on that page, or forget them in Settings.
10. **GitHub** (only when making your own copies): sign in with a short code, then choose *My personal account (<login>)* or *An organisation I belong to* and which one.
11. **Review and install.** Kora shows a dry-run plan first, then the install with live progress.

If an install fails, the panel keeps your answers (never the licence key or the database password; only the name of a
saved key) and offers "Continue where you left off" the next time. An install that stopped at a failed step and uses a
saved key can also carry on after Kora Panel was restarted ("Carry on"); the database administrator password, if you
typed one, is not kept and the install uses the automatic login or fails at that step again. A key you pasted without
saving it is only remembered while the panel runs. The wizard also checks free disk space before it begins.

## 6. Settings

- **Kora version:** check for a newer Kora and update it (same as `kora self-update`), or install a specific version.
- **Kora shortcuts:** the Start Menu / desktop shortcut that opens the panel (yours only, no administrator rights; same
  as `kora app install`). If the panel finds the old machine-wide Kora app from earlier versions (a copy in Program
  Files or /opt/kora, and shortcuts that ask for administrator rights) it offers **Tidy up old shortcuts**, also as a
  notice on the servers screen. That makes the new shortcuts first, so the old ones keep working until the end, then removes
  the old copy. Removing the old copy needs administrator rights: on Windows the panel asks Windows for permission
  (one prompt); on Linux it shows the command to run (`kora uninstall --app --yes`) or, with the helper installed, does it itself.
- **GitHub account:** sign in or see who you are signed in as.
- **Licence keys:** the keys saved on this computer: in your **system keychain** when this computer has one (Windows: protected for your
  account; Linux: GNOME Keyring, KWallet), otherwise in Kora's private data file (`store/kora.db`, readable only by you, not encrypted, like
  each server's `secrets.cfg`); each key says where it is kept, and a button turns the keychain on or off (`kora keys keychain`). A key that
  cannot be read from here (locked keychain, another session, deleted entry) says so and offers **Enter it again**; see [store.md](store.md). Add, rename, replace and delete (admins and this computer only); each
  shows its last 4 characters and the servers that use it. Deleting a key does not affect servers already installed with it
  (they have their own copy); an unfinished install that used it needs another key. Same as `kora keys`.
- **Remembered answers:** your Cfx.re name and GitHub owner, kept after first use in Kora's data file (nothing secret), with a
  button to forget each. Same as `kora profile`.
- **Uninstall Kora:** shows exactly what would be removed, item by item, and asks you to type `uninstall`. Server folders
  and databases are kept unless you tick them. Your saved licence keys and remembered answers are never removed from here;
  `kora uninstall` in a terminal asks about them (default: keep), or run `kora keys remove <name>` and `kora profile forget`.
- **Recent jobs:** updates, installs and the like, also from before the panel last restarted. A job that was running when
  the panel was closed shows as "stopped by a restart" with what to do next (an install that used a saved key can be carried on).
- **Activity log** (administrators only): who changed what, from this panel (this computer or another device, with sign-in name, provider
  and role) and from `kora` commands, with the result. Nothing secret is recorded; entries are kept 365 days. Same as `kora audit`.
- **About:** the version, platform, and links. The settings page makes no requests to other websites.

### The System check

**System check** in the side menu (or `kora check` in a terminal) lists what Kora needs on this computer, each line in plain
words with a button where something is wrong:

- **Git** and **GitHub CLI**: *Install it for me* downloads Kora's own copy into its own folder (a known, checksum-verified
  release from GitHub; no administrator rights; nothing else on the computer changes). If you already have Git or the
  GitHub CLI Kora uses yours. If the download fails (no internet, a proxy, GitHub busy) you get *Try again* and a link to
  install it by hand. Git on Linux is your system's package, which needs administrator rights (the helper does it, or the
  panel shows the command). `kora tools status` shows which copy is used and where it is; `kora tools install` sets them up.
- **GitHub sign-in**: *Sign in to GitHub* (or *Sign in again* when only some permissions are missing).
- **Database**: when a database server is installed but not running, *Start it for me*. Starting a service needs
  administrator rights: with the helper (Linux) it just works, otherwise the panel shows the command (`kora check
  --start-database`; Windows asks for administrator permission).
- **Administrator rights** and **Background service**: what Kora can do without asking, for your information.

The same buttons appear wherever an error offers them (the install wizard, the GitHub account card), and those errors link here.
`kora uninstall` also removes Kora's own copies of Git and the GitHub CLI.

## 7. Actions that need administrator rights

The panel runs as you, not as an administrator. A few things need more rights: creating or changing services, firewall
rules, update tasks and scheduled tasks, installing MariaDB, updating Kora itself.

**With Kora's administrator service there is no prompt at all.** The Windows installer (`.msi`) and the Linux package
(`.deb`, or `sudo kora service install` on other systems) install a small service that does exactly those parts for
the accounts an administrator allowed, and nothing else. One administrator approval at install time; after that the
panel installs and removes servers, changes schedules and updates Kora with no password and no terminal, from this
computer or from another device. The install wizard says so on its last screen ("Kora's administrator service will do
these parts"), and its steps show in the same progress list as everything else.

Who may use it: the account that installed Kora, plus anyone an administrator adds. On a computer shared by several
accounts each person's servers, services, firewall rules and databases are theirs alone.

```
kora service status          is it installed, running, and does it serve me?
kora service list            the accounts that may use it
kora service allow <account> (administrators) let another account use it
kora service deny <account>  (administrators) stop an account using it; its servers keep working
```

**Without the service** (you installed Kora just for yourself, without administrator rights) there is no automatic
path, and the panel never asks Windows or `sudo` by itself. When a step needs administrator rights it says so and
offers:

- **Install Kora's administrator service.** On Windows this starts the installer (one permission prompt, on the
  computer that runs Kora; the file is checked against its checksum first). On Linux it shows
  `sudo kora service install`.
- **Continue without the administrator parts:** the install carries on as you, without the firewall change, the
  background service and the daily update task (and with an existing database only).
- **Show me the command:** the command to run yourself from an administrator terminal.

If the service is installed but not turned on for your account, the message says so and gives the command an
administrator runs (`kora service allow <you>`). A step that failed halfway (for example the service was restarted)
can be tried again or, when it is safe, skipped, like any other install step.

Two things stay commands for an administrator terminal even with the service: removing the old machine-wide copy of
Kora and removing everything Kora set up on the machine (`kora uninstall`); Kora itself is removed in Apps & features
(Windows) or with `sudo apt remove kora` (Linux). Update tasks that run as the system account (a choice in the
schedule dialog) also stay a command: the service only makes update tasks that run as you.

**Updating Kora.** With the service installed the panel's Update button (and `kora self-update` from a terminal) is
applied by the service: it downloads the release, checks its checksum, keeps the old program, restarts itself on the
new one and rolls back by itself if the new one does not run.

**Upgrading from 0.7.0 or 0.6.x.** Run the new installer (MSI, `.deb`, or the one-liner, which offers it first). It
brings the old layout over without touching servers or data: the old Linux helper is removed (its account stays
allowed), update tasks that still ran with full administrator rights become unelevated, servers set up the old way
are recorded as yours, and the panel's "Tidy up old shortcuts" also sets aside your own earlier copy of Kora (kept as
`kora.previous`) and points the background panel at the installed one. To go back, install the earlier version's
installer; the old copy is still there as `.previous`.

## 8. Troubleshooting

**"cannot listen on 127.0.0.1:47120 ... is another panel running?"** You chose that port with `--port` and something
else uses it. Without `--port` Kora picks the next free port by itself, and if one of your own panels is already
running, `kora panel` shows its link instead of starting a second one.

**The browser did not open.** Kora prints the link; copy it into your browser. On a server without a desktop this is
expected (see [SSH](#starting-it-on-a-server-with-no-screen-ssh)). Use `--no-open` to stop Kora trying.

**The page says "The web UI was not built into this binary."** You are running a copy built from source without the web
UI. Release downloads include it. To build it yourself, see "Building from source" in the README (`cd web && npm ci
--ignore-scripts && npm run build`, then build Kora again).

**"Session expired, or not signed in".** Run `kora panel` again and open the new link, or paste the token. See
[Signing in](#2-signing-in).

**A Windows or Linux firewall prompt does not appear.** That is correct. The panel listens on `127.0.0.1` only, which no
other computer can reach, so the operating system has nothing to ask about. If you do get a firewall prompt, the panel is
not listening where it should; stop it and report it.

**The page loads but looks blank or shows errors after an update.** Restart the panel (`kora panel restart` for the background panel; in a window, Ctrl+C and `kora panel` again) and do a hard refresh (Ctrl+F5).
The page files are part of the program and are replaced with it.

**An action says it needs administrator rights.** See [section 7](#7-actions-that-need-administrator-rights).

**Something else.** The panel does everything the terminal does; try the matching command (`kora status`,
`kora update <server> --preview`) to compare. Report problems at
<https://github.com/KoraFiveM/kora-installer/issues>; for security problems follow [SECURITY.md](../SECURITY.md).

---

## 9. Optional: monitoring and alerts

The **Monitoring** tab shows, for each server: whether it is up, players online, CPU and memory of the server's processes,
disk space, port checks and the update badge, as charts you can view over 1, 6 or 24 hours. About 24 hours of history is
kept in memory and in a small capped file under the server's `.kora/` folder. CPU and memory per server are shown on
Linux; on Windows they show a dash for now.

**Alerts** tell you when something goes wrong: a banner in the panel, a Discord webhook, or a generic webhook (a URL that
receives a JSON message). Webhook addresses are stored but never shown back. Alerts are raised once per problem until it
clears: a server stopping unexpectedly (a stop from Kora or the CLI does not count), coming back up, low disk
space, high memory, and updates that need attention. The banner checks every 10 seconds. Alerts are kept in memory, so
they are lost when the panel restarts (the charts keep their history). Email alerts are not available yet.

**Keeping the panel running.** A panel run in a window stops when the window closes, so monitoring and alerts only work while it
runs. On a server with no screen `kora panel` offers to run it in the background for you; anywhere else, run `kora panel --install-service`. It starts when you sign in and needs no
administrator rights (Linux: a systemd user service; Windows: an entry in your Startup folder). `kora panel
--remove-service` removes it, and `kora panel --print-link` prints the sign-in link of the running background panel (the
link stays the same each time). `kora panel restart` restarts the background panel (open pages sign in again); a panel
running in a window is stopped there (Ctrl+C) and started again with `kora panel`. On a computer with a desktop it stays off unless you ask for it.

## 10. Optional: remote access

By default the panel is local only. You can reach it from your other computers and phones, safely.

### With Tailscale (recommended)

```sh
kora panel --tailscale
```

That one command:

- checks that Tailscale is running and connected here;
- makes **you** the panel's admin: the Tailscale account you are connected from (when your SSH session goes over
  Tailscale), otherwise the account that added this computer to Tailscale. Name someone else with
  `--admin you@example.com` (needed for a server added with an auth key and tags, which belongs to no person);
- keeps the background panel running and listening on `127.0.0.1` **and** on this computer's Tailscale addresses
  (never on its public address). It finds the addresses itself each time it starts and waits for Tailscale after a
  restart;
- with `ufw` on, lets Tailscale devices reach the panel's port on the Tailscale interface only (when run as root;
  otherwise it prints the `sudo ufw ...` command);
- prints the address to open, for example `http://myserver.tail1234.ts.net:47120`.

On any device signed in to Tailscale as an allowed account, open that address: you are signed in with your
Tailscale identity. The panel asks the Tailscale service on the server who is connecting; it never trusts anything
the browser sends about itself. Accounts that are not on the list are refused, even inside your tailnet.

Let other people in, each with a role (see below):

```sh
kora panel tailscale allow sam@example.com --role operator
kora panel tailscale list
kora panel tailscale remove sam@example.com     # their sessions end at once
kora panel tailscale off                          # back to this computer only (the list is kept)
```

The traffic is plain HTTP inside Tailscale's encrypted connection, so it cannot be read on the way. Use your
Tailscale ACLs if you want to limit which devices can reach the server at all.

Do not put the panel behind a reverse proxy or `tailscale serve`: every visitor would then look like the computer
itself. The panel refuses requests that came through a proxy.

### HTTPS: the connection to the panel

Plain HTTP is fine on this computer's own address and inside Tailscale (already encrypted). On a home network or a
public address the session cookie, the sign-in link and everything you type (licence keys) would cross the network in
clear, so the panel serves **HTTPS** there:

- the loopback address (`http://127.0.0.1:47120`) stays plain HTTP: no certificate warnings for the normal local case;
- any other address uses HTTPS on the **same port** as soon as there is a certificate, and says loudly when there is none.
  A browser that opens `http://` on that port from another computer is sent to `https://` (plain requests from this
  computer are still answered, so the launch link keeps working);
- `--allow-public` refuses to start without HTTPS (or a reverse proxy you told Kora to trust, below);
- the session cookie gets `Secure` over HTTPS; `Strict-Transport-Security` is sent only for a certificate you provided
  or Tailscale's (not for a self-signed one: a mistake there would lock you out of the name for a year); the "Open
  txAdmin" gateway ports, the live views and the console use the same HTTPS.

Where the certificate comes from, in order of preference (`kora panel tls ...`):

1. **Your own certificate:** `kora panel tls set --cert fullchain.pem --key privkey.pem`. The panel reads the files
   again when they change (certbot, a script), so renewing needs no restart. The key file must not be readable by other
   accounts (`chmod 600`); the panel refuses it otherwise and says the command.
2. **Tailscale:** `kora panel tls tailscale`. Kora asks `tailscale cert` for this computer's tailnet name and renews it
   before it ends. It needs MagicDNS and "HTTPS Certificates" turned on in the Tailscale admin console (DNS page): when
   Tailscale says no, its own reason is shown. Open the panel by its Tailscale **name**; the certificate is for that name
   (the `100.x` address gives a name mismatch). With `kora panel --tailscale` this is used automatically when it works.
3. **Self-signed:** `kora panel tls self-signed --name box.lan`. Kora makes a local certificate authority (once) and a
   server certificate from it for this computer's names and addresses (it is made again by itself before it ends, or when a
   name is added). You trust the **authority once**: `kora panel tls export-ca` saves it as `kora-local-ca.crt`, and the
   command prints how to install it on Windows, macOS, Linux, Firefox and phones. Compare the fingerprint it prints with
   what the browser shows (also in Settings > HTTPS) before you install it.

`kora panel tls status` shows what is in use (certificate, names, end date, fingerprint); `kora panel tls off` turns HTTPS
off again (not recommended); `kora panel tls renew` fetches or makes the certificate again now. The System check has an
HTTPS line, and a certificate in its last 14 days is warned about in Settings, the System check, the terminal and the
activity log.

**Public domain names with Let's Encrypt are not done by Kora.** Put a reverse proxy (Caddy, nginx) in front instead and
tell Kora to trust it:

```sh
kora panel proxy trust 127.0.0.1          # prints a secret and ready-to-paste Caddy and nginx snippets
```

Kora then accepts `X-Forwarded-For` and `X-Forwarded-Proto` only from that address **and** only on requests that carry
the shared secret (`X-Kora-Proxy-Secret`). The visitor's address (the right-most one that is not a proxy of yours) is what
the sign-in lockout counts; `X-Forwarded-Proto: https` makes the request count as secure; a proxied visitor is always a
remote one, never the person at the computer, and never gets a Tailscale identity. Without the secret, or from another
address, forwarded headers are refused as before. Add the public name to the allowed names (Settings > Remote access)
and turn remote access on. The "Open txAdmin" gateway ports are not proxied this way. `kora panel proxy forget` removes the trust.

### Other options

1. **SSH tunnel** (`ssh -L`, as in [section 1](#starting-it-on-a-server-with-no-screen-ssh)). Nothing is exposed.
2. **WireGuard or another private network:** start the panel with `kora panel --bind <private address> --remote`, set
   up HTTPS as above if it is not an encrypted network, and sign in with a passkey or one-time codes (below). Kora
   refuses public addresses and `0.0.0.0` unless you add `--allow-public`, which also needs HTTPS.

### Signing in from another computer: passkeys and one-time codes

A remote user used to sign in with a **token**: a secret in a link, valid until revoked, so whoever held the link *was*
that user. That is no longer the day-to-day credential.

1. On the computer that runs Kora: `kora panel user invite sam --role operator` (or **Invite person** in Settings > Remote
   access). It prints a **one-time link** (works once, for 30 minutes). That link is the only bearer link left.
2. Sam opens the link, and sets up **a passkey** (WebAuthn: the phone, laptop or security key holds a private key and
   asks for a fingerprint, face or PIN; nothing is replayable) or **a password and one-time codes** (any authenticator app;
   ten single-use recovery codes are shown once). Sam is signed in at the end.
3. From then on Sam signs in with the passkey (type the name, touch the device) or password + code.

**Passkeys need a name and a secure address.** A passkey belongs to the name it was registered on (the "relying party
ID"), and browsers allow passkeys only over `https://` (or `localhost`). So:

| how the panel is opened | passkeys |
|---|---|
| `https://box.tail1234.ts.net:47120` (Tailscale HTTPS) | yes, for `box.tail1234.ts.net` |
| `https://panel.example.com` (your domain, reverse proxy) | yes, for `panel.example.com` |
| `https://box.lan:47120` (a LAN name with a certificate) | yes, for `box.lan` |
| `https://192.168.1.5:47120` (a bare address) | **no**: an IP address cannot be a relying-party ID; use one-time codes, or give the panel a name |
| `http://...` | **no**: not a secure address |

Pick one name for remote access and register there: a passkey registered for one name does not work on another (the page
says so). People who cannot use a passkey (a bare address, an old browser) use **password + one-time codes**, which work
everywhere. Passwords are checked for strength (12 characters or more, not a common one, not the name) and stored as
argon2id hashes; a code is accepted for the current 30-second step and one either side, and **each step only once**
(a code seen over a shoulder cannot be replayed); five wrong passwords lock that account for 15 minutes (a passkey still
works, so a stranger cannot lock the owner out of their own device); wrong attempts from one address lock that address
out, as for tokens.

**Lost a phone or laptop?** On the computer that runs Kora: `kora panel user reset sam` (Settings: **Reset sign-in**). It
removes the credentials you choose (`--what passkeys|code|all|legacy-token`), signs every device of that person out at
once and prints a new one-time link. A user can also remove a lost passkey themselves from another device (**Your sign-in**
in Settings), which signs their other devices out.

**Your sign-in** (Settings, for people signed in remotely) lists your passkeys, one-time-code status and recovery codes,
and the **signed-in devices** (browser, address, when) with Sign out. Changing how you sign in asks for a fresh check first.

**A fresh check before the most sensitive actions.** For a session that signed in with a passkey or one-time codes, deleting
or unregistering a server, uninstalling Kora, updating Kora, console input, schedules and opening txAdmin ask for a passkey or
code check made in the last 10 minutes (the page shows a "Confirm it is you" box and then carries on). This comes on top of
the roles and the remote-actions switch, which are unchanged. Sessions: a remote passkey or code session ends after 8 hours
idle or 3 days in all (the computer's own and Tailscale sessions keep 12 hours / 7 days; an old token session at most 1 day).

**API tokens, for scripts.** `kora panel user api-token create backup-script --role operator --days 90` prints a token
once (stored only as a salted hash; optional expiry). It is sent as `Authorization: Bearer <token>` and **never** accepted in
an address (query or `?ks=`), never makes a browser session, and cannot sign anyone in to the page. Its role is its limit,
and deleting or uninstalling through it still needs `kora panel remote-actions on`. `api-token list` and `api-token revoke`
manage them; Settings > Remote access has the same.

**Upgrading from token links.** Nothing breaks on upgrade: existing users keep their token, which still signs them in to the
page and still works as a header for scripts. At their next sign-in the page suggests a passkey or one-time codes ("Replace
your token link"). Once a user has a passkey or one-time codes, their old token link is **not accepted for signing in any more**
(so a leaked old link cannot bypass them); it keeps working as a script header until the user ends it (**End my token link**)
or an administrator resets it. An administrator can turn on **Require a passkey or one-time codes for everybody** (Settings >
Remote access, or `PUT /remote`), which refuses the token link for signing in for everyone. New users never get a token.

**What a remote administrator can do depends on how they signed in.**

| | Tailscale admin | Passkey, code or token admin |
|---|---|---|
| Install a new server (start an install, resume a paused one) | yes | yes |
| Manage servers: start/stop, settings, resources, snapshots, ports, GitHub | yes | yes |
| Delete a server, unregister a server, uninstall Kora, remove the Kora app | yes | **off** until `remote actions` is on |
| Send console commands, update Kora, set or remove restart schedules | yes | **off** until `remote actions` is on |
| Open a server's folder, manage users and remote access, restart the panel | never (this computer only) | never (this computer only) |

An SSH tunnel is a local session: everything works. Viewer and operator roles are unchanged (no installs, no
destructive actions, whatever the setting).

Why the difference: a Tailscale sign-in is checked by the Tailscale service on this computer, so it names a real
person on a device they have signed in. A token is a secret in a link: it can be pasted into a chat or left in a
backup, and whoever holds it *is* that user (a passkey or one-time code cannot be copied that way, but the switch stays the same for now: what each role may do is unchanged). A leaked token that can only install can be undone by removing the
server it made. A leaked token that can delete cannot be undone, so deleting stays off for tokens until someone on
the computer that runs Kora turns it on. The typed-name confirmation for deleting always applies.

When a token session asks for one of those, the panel says why and shows the command. To allow them for token
administrators, run this on the computer that runs Kora (over SSH is fine):

```sh
kora panel remote-actions on       # off turns them off again; status shows the setting
```

or use the switch in Settings > Remote access from a local session. A running panel uses the new setting right away;
no restart is needed. Installing on Windows from afar still needs someone at the computer to accept the Windows
administrator prompt.

**A panel admin can do a lot on that computer**: install and delete servers, send console commands, update Kora. The panel itself does not run as root (since 0.6.1), but only make people you fully trust
admin; give others the operator or viewer role.

## 11. Optional: the hub (several machines)

Any panel can act as a hub: a single page that lists the servers of several machines (nodes). There is no central
service and nothing leaves your own machines.

Open **Fleet** in the sidebar to add a node: its address and a token. The token is one made on that node with
`kora panel user add <name> --role operator`, which is shown once. Adding nodes is only possible from a local admin
session. Nodes tell the hub which
version they run and which operations they support, so a hub on a newer version still works with an older node and
simply hides what the node cannot do. Treat node tokens like passwords, and reach nodes over Tailscale, WireGuard or an
SSH tunnel as described in [section 10](#10-optional-remote-access).

---

## 12. Security model in short

- **Local only by default.** The panel binds to `127.0.0.1`. Other computers cannot connect. Remote access listens
  only on private addresses (Tailscale with `--tailscale`); public addresses need `--allow-public`.
- **Identity from Tailscale, not the browser.** Tailscale sign-in asks the Tailscale service on the computer who is
  connecting, and only listed accounts get in, each with a role.
- **HTTPS off the loopback address, passkeys and one-time codes for people.** See sections above and `SECURITY.md`.
- **One-time sign-in links.** The link printed by `kora panel` holds a one-time code (it works once, for 10 minutes,
  and only on the computer that runs Kora), never the panel's long-lived token. It is exchanged for a session.
  Wrong codes and tokens are rate limited: on the computer itself, and per address for remote sign-ins (so a
  stranger guessing only locks themselves out). `kora panel --new-token` replaces the token.
- **A cookie is not enough.** Browsers send a cookie to every port of a host, so the panel's cookie also reaches
  txAdmin and the FiveM server's own web server, where any resource can log it. Each session therefore also has a
  secret that the page keeps for its own address and port and sends with every request (`X-Kora-Session`); the
  cookie alone is refused. The cookie (`kora_session_<port>`) is `HttpOnly` and `SameSite=Strict`.
- **txAdmin through the panel is a second door, guarded by the first.** The gateway port of a server's txAdmin (txAdmin's port + 10000) lets nobody in without a one-time ticket made by a signed-in administrator's panel session (cookie and session secret together); the gateway then follows that session (signing out, a removed user, a changed role or the remote-actions switch ending it, also for an open live console), never passes the panel's cookies or secret on to txAdmin, keeps two servers' txAdmin cookies apart, only ever connects to that server's own txAdmin on this computer, and refuses proxy headers, other `Host` and `Origin` values and WebSocket handshakes without a matching `Origin`. See `SECURITY.md`.
- **No proxies, unless you say so.** Requests that came through a reverse proxy or `tailscale serve` are refused (they connect from
  127.0.0.1, so everyone would look like the owner), except from the one proxy you told Kora to trust, with its secret.
- **Limits for remote sessions.** At most 8 live views per account and 64 in total. A remote session can only read a
  job its role could have started. The hub (fleet, other panels' servers) is local only. Alert webhooks set from
  another computer must point to the public internet.
- **Protection against other websites.** Every request must come with the panel's own address in the `Host` header
  (stops DNS rebinding). Changing requests must also carry a matching `Origin` and an `X-Kora` header (stops cross-site
  requests).
- **Strict page policy.** The page can only load its own files: no inline scripts, no other sites, no framing.
- **Secrets are write-only.** Licence keys, database passwords and webhook addresses are accepted but never sent back to
  the browser. Saved licence keys show only their name and last 4 characters, and live in the system keychain when there is one.
- **No elevated panel.** The panel runs as you (as root only when you are signed in as root, see [section 10](#10-optional-remote-access)). Privileged actions are a command you run, a helper that accepts only
  Kora's fixed actions (Linux), or on Windows a short-lived administrator copy of Kora started through the Windows
  permission prompt that runs only Kora's fixed administrator steps and exits (see SECURITY.md for how it is protected).
- **No telemetry.** The page makes no requests to other websites. Fonts and icons are built in.

Report a vulnerability privately; see [SECURITY.md](../SECURITY.md).

## Setup codes are gone

Earlier versions let the Kora website hand you a setup code for `kora install --code`. Setup codes were removed in 0.4.0.
`--code` still exists only so that old commands fail with a clear message (exit code 2). Instead, use:

- `kora panel`, then "Install a server".
- Command-line flags for scripts: `kora install --name ... --hostname ... --game-build ... --resources ...`
  (see `kora install --help`).
- `kora add-resources <server> --resource <ID>` to add resources.
