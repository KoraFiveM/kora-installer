# Kora Panel user guide

Kora Panel is the web page that comes with Kora Installer 0.4.0. It lets you install, manage and (in later sections of
this guide) monitor FiveM servers from your browser instead of the terminal. Everything the command line and the terminal
manager can do is also in the panel, with one exception: setup codes no longer exist (see [Setup codes](#setup-codes-are-gone)).

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

Run this on the computer that hosts your servers (Windows 10/11 or Server 2019+, or Debian 12+ / Ubuntu 22.04+):

```sh
kora panel
```

Kora starts a small web server, prints a line like `Kora Panel: http://127.0.0.1:47120/#token=...` and opens your
browser at that address. The panel keeps running in that terminal; press Ctrl+C to stop it. Closing the terminal also
stops it (servers you started are not affected: they run as services).

Options:

| Option | What it does |
|---|---|
| `--port <PORT>` | The port to listen on (default 47120). Use this if something else already uses 47120. |
| `--no-open` | Do not open a browser; print the address and sign-in link instead. This is automatic over SSH or when the computer has no desktop. |
| `--no-elevate` | Do not ask Windows or `sudo` for administrator permission (for developers and CI). |

Run `kora panel --help` to see the options your version supports.

### Starting it on a server with no screen (SSH)

On a headless Linux server there is no browser to open. The panel notices this (an SSH session, or no display) and prints
the link and a tunnel hint:

```
Kora Panel: http://127.0.0.1:47120/#token=...
No local desktop session detected; open the URL on your machine with:
  ssh -L 47120:127.0.0.1:47120 user@host
Press Ctrl+C to stop.
```

On your own computer, open a second terminal and run the `ssh -L` line it printed (replace `user@host` with your login).
Leave it open, then paste the full link, including the `#token=...` part, into your browser. The tunnel carries the page
over SSH, so nothing is exposed to the network.

## 2. Signing in

The link that `kora panel` prints contains a one-time secret (the token). When you open it:

1. The page sends the token to the panel, which checks it and gives your browser a session cookie.
2. The token is removed from the address bar straight away. The page never stores it.
3. From then on the cookie keeps you signed in.

**Do not share the link** while the panel is running; it is the password. It changes every time you start the panel.

### When the session expires

Sessions last until you stop the panel. If you restart `kora panel`, or open the panel in a new browser, you see
"Session expired, or not signed in". To fix it, run `kora panel` again and open the new link. You can also paste the token
(the part after `#token=`) into the box on that page.

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
| Updates | Preview what would change, take a snapshot, apply the update, choose what happens about restarts, and roll back to an earlier snapshot. Same as `kora update`. |
| Resources | The installed resources and the catalogue of ones you can add (compatibility, bridges, creators). Dependencies are added for you. Same as `kora add-resources`. |
| Schedule | Development servers only: when the server updates itself. Changing it needs administrator rights. Same as `kora schedule`. |
| Notifications | The Discord webhook for update reports: set, test, remove. The address is never shown back to you. Same as `kora webhook`. |
| Ports | Change the game and txAdmin ports. The server must be stopped. Kora checks the ports are free. Firewall rules are not changed: open the new ports and close the old ones yourself. |
| GitHub | Development servers only: sign in to GitHub, report an issue about a resource, or open a pull request. |
| Danger zone | Delete the server. You choose whether to keep the files and the database, then type the server's name to confirm. |

## 5. Installing a server (the wizard)

"Install a server" walks through the same steps as `kora install`:

1. **Mode:** production (pinned releases) or development (forks on your GitHub account or organisation).
2. **Name and folder:** the server name, and where it lives. A suggested name and folder are filled in.
3. **Ports:** game port (30120 by default) and txAdmin port (40120). Kora checks they are free and suggests the next free
   ones if they are not.
4. **Build:** the recommended FXServer build, or one you choose (advanced).
5. **Resources:** pick what to install. Dependencies and the core resources are always included.
6. **Database:** use the MariaDB already on the machine, let Kora install it, or skip it. "Test login" checks the admin
   password before anything starts.
7. **Options:** start the server when finished, service, firewall rule, the Kora app and desktop shortcut.
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
- **Kora app:** install or remove the `kora` command and Start Menu shortcut (same as `kora app`).
- **GitHub account:** sign in or see who you are signed in as.
- **Uninstall Kora:** shows exactly what would be removed, item by item, and asks you to type `uninstall`. Server folders
  and databases are kept unless you tick them.
- **About:** the version, platform, and links. The settings page makes no requests to other websites.

## 7. Actions that need administrator rights

The panel runs as you, not as an administrator. A few things need more rights: creating or changing services, firewall
rules, update timers and scheduled tasks, installing MariaDB, and installing or removing the Kora app.

When you ask for one of these, the panel does one of two things:

- **Shows the exact command to run.** A message appears with a command such as `sudo kora ...` (Linux) or an elevated
  `kora ...` (Windows). Copy it into a terminal; the command asks for administrator permission itself. When it
  finishes, return to the panel and refresh.
- **Uses the helper, if it is installed (Linux only for now).** The helper is a small system service, installed once with
  `sudo kora helper install`, that runs those same actions for the panel without a prompt each time. It accepts only the fixed list of privileged actions Kora already has and checks which user is asking. Check it with `kora helper status`, remove it with `kora helper uninstall`.

The Settings page says when the helper is installed. On Windows the panel always shows the command to run.

## 8. Troubleshooting

**"cannot listen on 127.0.0.1:47120 ... is another panel running?"** The port is taken, usually by a panel you
left running. Stop that one (Ctrl+C in its terminal) or choose another port: `kora panel --port 47121`.

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

**The page loads but looks blank or shows errors after an update.** Restart `kora panel` and do a hard refresh (Ctrl+F5).
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
clears: a server stopping unexpectedly (a stop from Kora, the CLI or the TUI does not count), coming back up, low disk
space, high memory, and updates that need attention. The banner checks every 10 seconds. Alerts are kept in memory, so
they are lost when the panel restarts (the charts keep their history). Email alerts are not available yet.

**Keeping the panel running.** `kora panel` stops when you close its terminal, so monitoring and alerts only work while it
runs. To keep it running in the background, run `kora panel --install-service`. It starts when you sign in and needs no
administrator rights (Linux: a systemd user service; Windows: an entry in your Startup folder). `kora panel
--remove-service` removes it, and `kora panel --print-link` prints the sign-in link of the running background panel (the
link stays the same each time). It is off unless you ask for it.

## 10. Optional: remote access

By default the panel is local only. You can reach it from another computer, but do it safely.

**Safest options first:**

1. **SSH tunnel** (`ssh -L`, as in [section 1](#starting-it-on-a-server-with-no-screen-ssh)). Nothing is exposed.
2. **Tailscale or WireGuard.** The panel listens on the private address only and the private network does the
   encryption. Never bind it to a public address.

To listen on a specific address, start the panel with `kora panel --bind <ip> --remote`. For example, with Tailscale,
use the machine's Tailscale address.

**The page is plain HTTP.** Without Tailscale, WireGuard or an SSH tunnel around it, anyone on the network path can read
and change what you send, including the sign-in token. Do not use plain HTTP over the internet or on a shared network.

**Users and roles.** Remote sign-in uses named users: `kora panel user add`, `kora panel user list`,
`kora panel user remove`. Each has a role:

- **viewer:** can look at server status and lists, the fleet and charts; cannot change anything, and cannot read logs or
  the console (they can contain player names and addresses).
- **operator:** a viewer, plus logs and console output, start/stop/restart, update preview and apply, ensuring resources.
- **admin:** an operator, plus settings (ports, webhooks, resources, snapshots, GitHub).

Managing remote access and users is only possible from a local session, never remotely. Remote sessions end after 12
hours idle or 7 days in total, and at once when the user is removed or their role changes.

With Tailscale, the panel can recognise you by your Tailscale identity (it asks the local Tailscale service who is
connecting; it never trusts identity headers sent by the browser). Requests from the machine itself always need the token.

**Destructive actions are off for remote sessions by default.** Deleting a server, uninstalling and sending console input
only work from a local session unless an admin switches them on for that machine. Even then you must type the server's
name to confirm.

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

- **Local only by default.** The panel binds to `127.0.0.1`. Other computers cannot connect.
- **A secret link.** A random token is generated at every start and given to you in the link. It is exchanged for a
  session cookie (`HttpOnly`, `SameSite=Strict`). Wrong tokens are rate limited.
- **Protection against other websites.** Every request must come with the panel's own address in the `Host` header
  (stops DNS rebinding). Changing requests must also carry a matching `Origin` and an `X-Kora` header (stops cross-site
  requests).
- **Strict page policy.** The page can only load its own files: no inline scripts, no other sites, no framing.
- **Secrets are write-only.** Licence keys, database passwords and webhook addresses are accepted but never sent back to
  the browser.
- **No elevated panel.** The panel runs as you. Privileged actions are a command you run, or a helper that accepts only
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
