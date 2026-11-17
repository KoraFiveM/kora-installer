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
`kora manager` does the same, and `kora install` (with no options) opens the install page.

What `kora` does when things are not simple:

| Situation | What happens |
|---|---|
| The panel is already running | It is opened; nothing is started twice. |
| Port 47120 is taken by another program | The next free port up to 47140 is used (the link shows it). |
| No background service is possible (for example a container with no user session) | The panel runs in that window instead; closing the window stops it. |
| A server with no screen (SSH) | Nothing opens. Kora prints the sign-in link, a ready-to-paste `ssh -L` command to run on your own computer first, and the Tailscale option (`kora panel --tailscale`). |
| Typed with `sudo` (Linux) | Kora runs again as the person who typed `sudo` (the panel never runs as root). |
| Typed as root with no `sudo` user (Linux), or in an "administrator" window (Windows) | A running panel is opened. Otherwise Kora explains what to do (log in as a normal user / open Kora from the Start menu or a normal PowerShell window). Nothing is changed. |

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

## 4. A server's tabs

| Tab | What it is for |
|---|---|
| Overview | State, ports, the update badge, start / stop / restart, open the server folder (only when you are on that computer). |
| Console | Live server output and a box to send commands. The first time, Kora offers to add a small resource called `kora_console`; it loads on the next server start. |
| Logs | Pick a log file and read its latest lines. |
| Resources | **Updates** at the top: check for newer versions, pick what to update, choose what happens about restarts, and roll back to an earlier snapshot (same as `kora update`). Below it the installed resources, each with buttons to check it for updates and to start, stop or restart it on a running server (needs the live console from the Console tab), and the catalogue of resources you can add (same as `kora add-resources`). What the tab shows stays when you open another tab and come back. |
| Schedule | Development servers only: when the server updates itself. Changing it needs administrator rights. Same as `kora schedule`. |
| Notifications | The Discord webhook for update reports: set, test, remove. The address is never shown back to you. Same as `kora webhook`. |
| Ports | Change the game and txAdmin ports. The server must be stopped. Kora checks the ports are free. Firewall rules are not changed: open the new ports and close the old ones yourself. |
| GitHub | Development servers only: sign in to GitHub, **change a resource's branch or try a pull request** (see below), report an issue about a resource, or open a pull request. |
| Danger zone | Delete the server. You choose whether to keep the files and the database, then type the server's name to confirm. |

### Branches and pull requests (Development servers)

On a Development server every resource is a clone of your own fork. The GitHub tab's **Branches and pull requests** card lets you move a resource onto another version of its code without using git yourself. The same operations are `kora branch ...` and `kora pr ...` in a terminal.

- **See where each resource is.** Each resource shows "Normal version (dev)", "Branch fix-hud", "Trying PR #12" or "A single commit", plus how many files you changed and how many sets of changes are kept aside. The Resources tab shows the same next to the name.
- **Switch to a branch.** Pick it from the list (branches already in the folder, on your fork, on the original repository; "Refresh list" asks GitHub for the newest) and press Switch.
- **Try a pull request.** Paste its number or its GitHub address, or pick one from the list. The list holds the open pull requests of the original repository *and* of your fork, each marked "Original repository" or "Your fork" with its title, number and author. The pull request is downloaded onto a branch called `pr/<number>`; trying it again later fetches its new commits (fast-forward only; if the author rewrote it, it goes on a new branch and your earlier copy stays). A pull request that was merged or closed, a number that does not exist, an address on some other repository and a pull request whose fork was deleted each get a plain explanation.
- **Back to the normal version** puts the resource back on its normal branch (the one Kora updates). In the bulk section, "Back to the normal version, for all" does it for every resource.
- **Several resources at once.** Type a branch name, "Check which resources have it" shows who has it and who does not, then "Switch the N that have it" moves those and leaves the others as they are.
- **Your own changes are never lost.** A resource with changes you have not saved is not touched until you choose: *Keep my changes aside* (recommended), *Throw my changes away* (asked a second time, with a warning that it cannot be undone) or *Cancel*. Kept-aside changes are listed in that resource's window with *Bring my changes back*; if they no longer fit, nothing changes and they stay kept aside. In git they are an ordinary stash starting with "Kora:", so `git stash list` in the resource folder shows them too. Kora never deletes a stash, not even after bringing the changes back.
- **After a switch** the files on disk have changed but a running server still has the old version loaded. Kora says so and offers "Restart <resource> now" (needs the live console) or tells you to restart the server.

What updates do with a switched resource: **they leave it alone.** The update check shows it as "Not updated: on another branch" with the reason ("on branch fix-hud", "trying pull request #12"), the scheduled daily update skips it with the same words, and neither counts it as a problem or reports it "up to date". Nothing is ever pulled into your branch. Going back to the normal version makes updates work for it again. Snapshots record each resource's branch; a rollback first returns the resource to the branch it was on when the snapshot was taken and only then resets it, so it never rewrites a branch you moved to afterwards. Switching waits while an update is running (and the other way round). Production servers have none of this: they stay on their pinned releases.

Sign-in: listing and trying pull requests uses the GitHub CLI (`gh`); if you are not signed in, or `gh` is missing, the panel offers the usual sign-in and install buttons. Switching between branches that are already downloaded needs no GitHub sign-in. Switching branches needs the **admin** role, and from another computer the remote-actions switch, because it changes the code the server runs.

## 5. Installing a server (the wizard)

"Install a server" walks through the install steps (`kora install` with options does the same without asking):

1. **Mode:** production (pinned releases) or development (forks on your GitHub account or organisation).
2. **Name and folder:** the server name, and where it lives. A suggested name and folder are filled in.
3. **Ports:** game port (30120 by default) and txAdmin port (40120). Kora checks they are free and suggests the next free
   ones if they are not.
4. **Build:** the recommended FXServer build, or one you choose (advanced).
5. **Resources:** pick what to install. Dependencies and the core resources are always included.
6. **Database:** use the MariaDB already on the machine, let Kora install it, or skip it. "Test login" checks the admin
   password before anything starts.
7. **Options:** the background service and the firewall rule (the server is started when the install finishes).
8. **Licence key:** your Cfx.re licence key. It is sent to Kora on this computer only and is never shown again.
9. **Your Cfx.re account:** type your Cfx.re username and press "Check". Kora asks forum.cfx.re (only the username is sent)
   which account owns it, and that account becomes the txAdmin admin, so you sign in to txAdmin with Cfx.re. Leave it
   empty to skip: Kora then creates a local txAdmin admin with a generated password saved in the server folder.
10. **GitHub** (development mode only): sign in with a short code and choose where the forks go.
11. **Review and install.** Kora shows a dry-run plan first, then the install with live progress.

If an install fails, the panel keeps your answers (never the licence key or the database password) and offers "Continue
where you left off" the next time. The wizard also checks free disk space before it begins.

## 6. Settings

- **Kora version:** check for a newer Kora and update it (same as `kora self-update`), or install a specific version.
- **Kora shortcuts:** the Start Menu / desktop shortcut that opens the panel (yours only, no administrator rights; same
  as `kora app install`). If the panel finds the old machine-wide Kora app from earlier versions (a copy in Program
  Files or /opt/kora, and shortcuts that ask for administrator rights) it offers **Tidy up old shortcuts**, also as a
  notice on the servers screen. That makes the new shortcuts first, so the old ones keep working until the end, then removes
  the old copy. Removing the old copy needs administrator rights: the panel shows the command to run
  (`kora app uninstall --yes`) or, with the helper installed, does it itself.
- **GitHub account:** sign in or see who you are signed in as.
- **Uninstall Kora:** shows exactly what would be removed, item by item, and asks you to type `uninstall`. Server folders
  and databases are kept unless you tick them.
- **About:** the version, platform, and links. The settings page makes no requests to other websites.

## 7. Actions that need administrator rights

The panel runs as you, not as an administrator. A few things need more rights: creating or changing services, firewall
rules, update timers and scheduled tasks, installing MariaDB, and removing the old machine-wide Kora app.

When you ask for one of these, the panel does one of two things:

- **Shows the exact command to run.** A message appears with a command such as `sudo kora ...` (Linux) or an elevated
  `kora ...` (Windows). Copy it into a terminal; the command asks for administrator permission itself. When it
  finishes, return to the panel and refresh.
- **Uses the helper, if it is installed (Linux only for now).** The helper is a small system service, installed once with
  `sudo kora helper install`, that runs those same actions for the panel without a prompt each time. It accepts only the fixed list of privileged actions Kora already has and checks which user is asking. Check it with `kora helper status`, remove it with `kora helper uninstall`.
  Be aware of what it means: with the helper installed, your account (and any program running as you) has the same
  power as passwordless `sudo` for these Kora actions, so only install it on an account you keep safe. Through the
  helper, servers are installed only into a new or empty folder in your home folder or in `/home/kora`, never from
  another catalogue, and Kora is only updated to a newer version; for anything else run the command yourself with `sudo`.

The Settings page says when the helper is installed. On Windows the panel always shows the command to run.

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

### Other options

1. **SSH tunnel** (`ssh -L`, as in [section 1](#starting-it-on-a-server-with-no-screen-ssh)). Nothing is exposed.
2. **WireGuard or another private network:** start the panel with `kora panel --bind <private address> --remote` and
   sign in with a user token (below). Kora refuses public addresses and `0.0.0.0` unless you add `--allow-public`,
   because the page is plain HTTP: without an encrypted network around it, anyone on the way could read and change
   what you send, including sign-in tokens.

**Users and roles.** Besides Tailscale accounts, remote sign-in can use named users with a token each:
`kora panel user add <name> --role <role>`, `kora panel user list`, `kora panel user remove <name>`. Each Tailscale
account or user has a role:

- **viewer:** can look at server status and lists and charts; cannot change anything, and cannot read logs or
  the console (they can contain player names and addresses).
- **operator:** a viewer, plus logs and console output, start/stop/restart, update preview and apply, ensuring resources.
- **admin:** an operator, plus settings (ports, webhooks, resources, snapshots, GitHub).

Managing remote access and users from the panel is only possible from a local session, never remotely; the
`kora panel tailscale`, `kora panel user` and `kora panel remote-actions` commands work on the server at any time. Remote sessions end after 12
hours idle or 7 days in total, and at once when the account is removed or its role changes. The sign-in link from
`kora panel` only works on the computer itself (or through an SSH tunnel), never remotely.

**Destructive actions are off for remote sessions by default.** These only work from a local session, even for a
remote admin: deleting a server, unregistering it (removing it from Kora's list), uninstalling Kora, removing the Kora
app, sending console commands, installing a server, updating Kora, and setting or removing restart schedules. To allow
them for remote admins, run this on the computer that runs Kora:

```sh
kora panel remote-actions on       # off turns them off again; status shows the setting
```

or use the switch in Settings > Remote access from a local session. A running panel uses the new setting right away;
no restart is needed. Even when it is on you must type the server's name to delete it.

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
- **One-time sign-in links.** The link printed by `kora panel` holds a one-time code (it works once, for 10 minutes,
  and only on the computer that runs Kora), never the panel's long-lived token. It is exchanged for a session.
  Wrong codes and tokens are rate limited: on the computer itself, and per address for remote sign-ins (so a
  stranger guessing only locks themselves out). `kora panel --new-token` replaces the token.
- **A cookie is not enough.** Browsers send a cookie to every port of a host, so the panel's cookie also reaches
  txAdmin and the FiveM server's own web server, where any resource can log it. Each session therefore also has a
  secret that the page keeps for its own address and port and sends with every request (`X-Kora-Session`); the
  cookie alone is refused. The cookie (`kora_session_<port>`) is `HttpOnly` and `SameSite=Strict`.
- **No proxies.** Requests that came through a reverse proxy or `tailscale serve` are refused (they connect from
  127.0.0.1, so everyone would look like the owner).
- **Limits for remote sessions.** At most 8 live views per account and 64 in total. A remote session can only read a
  job its role could have started. The hub (fleet, other panels' servers) is local only. Alert webhooks set from
  another computer must point to the public internet.
- **Protection against other websites.** Every request must come with the panel's own address in the `Host` header
  (stops DNS rebinding). Changing requests must also carry a matching `Origin` and an `X-Kora` header (stops cross-site
  requests).
- **Strict page policy.** The page can only load its own files: no inline scripts, no other sites, no framing.
- **Secrets are write-only.** Licence keys, database passwords and webhook addresses are accepted but never sent back to
  the browser.
- **No elevated panel.** The panel runs as you (as root only when you are signed in as root, see [section 10](#10-optional-remote-access)). Privileged actions are a command you run, or a helper that accepts only
  Kora's fixed actions.
- **No telemetry.** The page makes no requests to other websites. Fonts and icons are built in.

Report a vulnerability privately; see [SECURITY.md](../SECURITY.md).

## Setup codes are gone

Earlier versions let the Kora website hand you a setup code for `kora install --code`. Setup codes were removed in 0.4.0.
`--code` still exists only so that old commands fail with a clear message (exit code 2). Instead, use:

- `kora panel`, then "Install a server".
- Command-line flags for scripts: `kora install --name ... --hostname ... --game-build ... --resources ...`
  (see `kora install --help`).
- `kora add-resources <server> --resource <ID>` to add resources.
