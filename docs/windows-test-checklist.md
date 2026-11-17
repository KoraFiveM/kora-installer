# Windows test checklist (v0.7.0)

A manual pass for the person releasing Kora, on a **real Windows machine**. Nobody could run the Windows code while it
was written: it was compiled and unit-tested on Linux, and only the CI smoke scripts (`scripts/smoke/`) have run it on a
GitHub Windows runner, which is an administrator account with no real desktop. Everything below is what those cannot
cover: shortcuts, Explorer, UAC, a real browser, a normal user's permissions.

Time: about 30 to 45 minutes for sections 1 to 6 on a clean Windows 11 machine, then section 8 to clean up. Section 7 (the
upgrade from 0.6.x) is an extra 10 minutes on a machine that has 0.6.x. The section "Administrator prompt (UAC)" belongs
to the elevated worker and has its own list. Tick a box when the step does what it says. When it
does not, stop on that step, write down its number and collect the items under **If not, copy back**.

## 0. Before you start

- [ ] A **clean Windows 11** machine or VM (a snapshot you can go back to is ideal). Sign in with an ordinary account
  that is allowed to approve UAC prompts (the usual first account is). Do **not** sign in as the built-in Administrator.
- [ ] A **normal** PowerShell window means one you opened from the Start menu without "Run as administrator". An
  **administrator** window is one opened with "Run as administrator". Steps say which to use.
- [ ] The release to test. Once it is published, use the one-line installer from the README (step 1). Before that, use
  the draft release: publish it as a **pre-release** first, or point the installer at the draft's files with
  `$env:KORA_RELEASE_BASE='https://github.com/KoraFiveM/kora-installer/releases/download/v0.7.0'` (https only, and the
  files must be downloadable without signing in).
- [ ] A real **Cfx.re licence key** (for the steps that start the server). A made-up key such as
  `cfxk_FAKEFAKEFAKEFAKEFAKE_test` installs fine but the game server will not start properly.
- [ ] A **GitHub account for testing** (a throw-away one or an organisation). The Development install in step 31 creates
  one fork per resource on it: delete them afterwards if you do not want them.
- [ ] Free disk: about 10 GB. Free ports 30120, 40120, 3306 (or let the wizard pick others).
- [ ] An empty text file open to paste into.

**What to copy back for any failure** (name it "step N"):
1. A screenshot of the panel or window, with the whole message visible.
2. The panel's **Show details** text (or **Copy details**), if the message has one.
3. The output of these, run in a normal PowerShell window: `kora --version`, `kora app status`, `kora tools status`,
   `kora check`.
4. For a failed install: the file `<server folder>\.kora\install.log`.
5. The contents of the Kora folder listing: `Get-ChildItem "$env:LOCALAPPDATA\Kora" -Recurse -Depth 2 | Select-Object FullName`.
   (Do **not** paste `secrets.json`, `secrets.cfg`, or anything that shows a licence key.)

## 1. Install and shortcuts (no administrator rights)

1. [ ] **The one-line installer.** In a **normal** PowerShell window run the Windows line from the README:
   `irm https://github.com/KoraFiveM/kora-installer/releases/latest/download/install.ps1 | iex`
   (adding `-Version`/`KORA_VERSION` if the release is not the newest stable one).
   *Should happen:* it downloads, prints "Checksum OK", "Installed kora to `C:\Users\<you>\AppData\Local\Kora\bin\kora.exe`",
   "Added ... to your PATH", then "Starting Kora Panel ...", your browser opens on the Kora panel and you are signed in,
   and the window ends with "Open Kora again any time from the Start Menu...". **No UAC prompt appears at any point.**
   SmartScreen may warn about the unsigned `.exe`: "More info" then "Run anyway" is expected for now.
   *If not, copy back:* the whole PowerShell output.
2. [ ] **The panel opened into the install wizard** (no servers yet). Do not install anything yet. Close nothing.
3. [ ] **Per-user shortcuts.** Open the Start menu and search "Kora": there is a **Kora** shortcut. There is a **Kora**
   shortcut on the Desktop too. Right-click the Start menu one > Open file location, then right-click the shortcut >
   Properties > Shortcut tab > Advanced: **"Run as administrator" is NOT ticked**; "Run" is *Minimised*.
   `Get-ChildItem "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Kora.lnk"` exists.
   *If not, copy back:* `kora app status` and a screenshot of the Properties dialog.
4. [ ] **Nothing machine-wide was made.** `Test-Path "$env:ProgramFiles\Kora"` is `False`. In Settings > Apps > Installed
   apps there is no "Kora" entry. `[Environment]::GetEnvironmentVariable('Path','Machine')` does not mention Kora.
5. [ ] **Start-up entry.** The file `$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Startup\Kora Panel.vbs` exists
   (it keeps the panel running after sign-in).
6. [ ] **Open a new normal PowerShell window** and run `kora --version`: it prints `kora 0.7.0`. (The new `PATH` entry
   works.)

## 2. Opening Kora in every way

7. [ ] **Double-click the Desktop shortcut.** The panel opens in your browser. A black console window must **not**
   flash up or stay open (the shortcut runs minimised). There is no UAC prompt. A second click does not start a second
   panel: it opens a new tab on the same one.
   *If not, copy back:* what you saw, the address in the browser bar (the port).
8. [ ] **Double-click `kora.exe`** itself (in `%LOCALAPPDATA%\Kora\bin`). The panel opens. Note whether a console window
   stays open and what it says (a short message is fine; a window that asks for a key press or sits for a long time is
   worth a note).
9. [ ] **Bare `kora` in a normal PowerShell window.** The panel (already running) is just opened: it prints a short
   line, opens the browser, returns to the prompt. `kora panel --print-link` prints a sign-in link.
10. [ ] **The panel starts in the background by itself.** Open Task Manager > Details, end every `kora.exe` (this stops
    the panel; servers are not affected). In a normal PowerShell window run `kora`. *Should happen:* within a few
    seconds it prints that it started the panel, opens the browser, and gives the prompt back (it does not stay
    running in the window). A `kora.exe` appears again in Task Manager.
    *If not, copy back:* the window output and `kora panel --print-link`.
11. [ ] **Sign out of Windows and back in** (or restart). A minute after signing in, `kora` opens the panel without
    starting anything new, or the panel is already running (Task Manager shows `kora.exe`, run by your user).
12. [ ] **Bare `kora` from an administrator window, panel already running.** Open PowerShell with "Run as
    administrator", run `kora`. *Should happen:* the running panel opens in your browser, no error, nothing changed.
13. [ ] **Bare `kora` from an administrator window, panel NOT running.** End every `kora.exe` in Task Manager again, then in
    the administrator window run `kora`. *Should happen:* "Kora Panel does not run as administrator, so Kora is
    starting it as your normal user", and the panel opens. In Task Manager > Details, add the column **Elevated** and
    check that `kora.exe` is **not** elevated and runs under your user name. (If Kora instead says the window is
    running as administrator and tells you to open Kora from the Start menu, note that: it is the fallback when the
    start-up entry from step 5 is missing.)
14. [ ] **`kora panel` from an administrator window** refuses to start the panel ("does not run elevated") and says what to
    do. Nothing is left running as administrator.

## 3. System check, Kora's own Git and GitHub CLI (no `winget`)

15. [ ] **System check page.** In the panel open **System check**. On a clean machine Git and the GitHub CLI show as
    not installed, each with **Install it for me**; "Administrator rights" says Kora runs with normal rights;
    "Background service" says the panel runs in the background. Nothing mentions `winget`.
    *If not, copy back:* a screenshot and `kora check`.
16. [ ] **Install Git and the GitHub CLI for me.** Press the button for Git: a progress bar runs, then it turns to
    "Installed". Do the same for the GitHub CLI. **No UAC prompt, no "close this window" message.**
    - `kora tools status` shows both as "Kora's own copy", under `%LOCALAPPDATA%\Kora\tools\...` (copy the two paths).
    - In a **new** normal PowerShell window `where.exe git` and `where.exe gh` find nothing (Kora did not change the
      `PATH`), yet the System check still lists both as installed.
    - `Get-ChildItem "$env:LOCALAPPDATA\Kora\tools" -Force` shows no leftover `.dl-*` or `.stage-*` folders.
    *If not, copy back:* the Show details text, `kora tools status`, the folder listing.
17. [ ] **When the download fails.** *(Optional, 3 minutes.)* Run `kora uninstall --dry-run`: it lists Kora's own copies
    of Git and the GitHub CLI as removable and changes nothing. Then run `kora uninstall` and answer **no** to every
    question except removing Kora's own copies (**yes**). Turn off Wi-Fi or unplug the network and press **Install it for
    me**: you get a plain message with **Try again** and a link to install it by hand. Reconnect and press **Try again**:
    it works. (Do this step before step 18, or sign in to GitHub again afterwards.)
18. [ ] **GitHub sign-in with Kora's own `gh`.** On the System check page press **Sign in to GitHub**. A code is shown
    and the browser opens GitHub's device page; enter the code, approve. The page then shows you as signed in with your
    GitHub name. *Not expected:* a Windows console window asking you questions, or the message "gh is not installed".
    - `& "$env:LOCALAPPDATA\Kora\tools\gh\bin\gh.exe" auth status` (adjust the path to the one in step 16) says
      "Logged in".
    *If not, copy back:* the Show details text and the output above.

## 4. Saved licence keys and `secrets.json`

19. [ ] **Add a key in Settings.** Settings > Licence keys > add one named "Main" with your real key. The list shows
    "Main" and only the **last four characters**. The full key is nowhere on the page (also not in "Show details").
20. [ ] **`secrets.json` is private to you.** In a normal PowerShell window run
    `icacls "$env:LOCALAPPDATA\Kora\secrets.json"`. *Should happen:* exactly one entry, your own account
    (`<MACHINE>\<you>:(R,W)`); **no** `BUILTIN\Users`, `Everyone`, `Authenticated Users`, and no `(I)` inherited
    entries. You can still open the file in Notepad (do not paste its contents anywhere).
    *If not, copy back:* the `icacls` output only.
21. [ ] **Replace, rename and delete work on that private file.** In Settings > Licence keys rename "Main" to "Live", add a
    second key "Test" (any text of the right shape, for example `cfxk_` followed by 20 letters/digits, then `_` and
    four more), replace "Test" with another, delete "Test". Every action succeeds with no "access denied" and
    `icacls` on the file still shows only your account. (The file is rewritten through a temporary file each time; this
    is the one place Windows permissions could get in the way.)
    *If not, copy back:* the error text and the `icacls` output.
22. [ ] **`kora keys list`** in a normal window shows the same names and last four characters, never a full key.

## 5. Install a Production server from the panel

23. [ ] **Start the wizard** (Servers > Install a server, or the first-run page). Choose **Production**. Pick the saved key
    "Live" from the list (do not paste it). Name the server `test prod`. Open **Change** next to the folder and confirm
    the suggested folder is `C:\Kora\test-prod` (or similar under `C:\Kora\`). Now **change it to a path with a
    space**: `C:\Kora Test\Prod Server`. Port checks are shown before the Install button. Do not tick "Open the firewall
    to txAdmin".
24. [ ] **Install.** Press Install. Watch the live steps.
    - Note **every** UAC prompt that appears and at which step (the service, the firewall rule and installing a database
      are the steps that need administrator rights). If your build has the elevated worker there should be **one**
      prompt (see the section "Administrator prompt (UAC)"); if the panel instead shows a message with a command to run as administrator, copy it, run it
      in an administrator window, and press **Try this step again**.
    - If a step fails the install must **stop and wait** with **Try this step again**, and **Skip this step** only for
      the firewall, schedule and start steps. Try "Try this step again" once on whatever fails.
    - *Should happen:* all steps finish, the last page offers Open txAdmin, Copy connect address and Stop server.
    *If not, copy back:* the Show details text, `<server folder>\.kora\install.log`, a screenshot of the step list.
25. [ ] **The server runs as a service.** `Get-Service kora-*` shows a running service for it. Open txAdmin from the
    button: the txAdmin page loads (it asks you to sign in with Cfx.re, or shows the local admin: either is fine).
26. [ ] **Open folder.** On the server's Overview press **Open folder**: Explorer opens on `C:\Kora Test\Prod Server`
    (the space does not break it) and **Copy path** is not shown (you are at this computer). The install path is shown
    next to the button.
    *If not, copy back:* what Explorer showed, a screenshot.
27. [ ] **Copy connect address** puts `<this PC's address>:30120` on the clipboard (paste it into your text file).
28. [ ] **Change the licence key on the running service.** Overview > **Change licence key** > pick "Live" again (or add
    "Test" and pick that). *Should happen:* Kora says whether a restart is needed and **offers it** because the
    server runs; press the restart and the service comes back (`Get-Service kora-*` is Running again).
    - `icacls "C:\Kora Test\Prod Server\secrets.cfg"` still lists the service account (`NT SERVICE\kora-...`) and your
      account, and **no** `Users`/`Everyone`.
    - Open `secrets.cfg` in Notepad: the `sv_licenseKey` line holds the key you chose. (Do not paste it anywhere.)
    - The key appears in no log under `C:\Kora Test\Prod Server\.kora\`: search with
      `Select-String -Path "C:\Kora Test\Prod Server\.kora\*" -Pattern "<the first 8 characters of the key>"`
      and expect no hits.
    *If not, copy back:* the Show details text, the `icacls` output.
29. [ ] **Stop, start and restart** the server from the panel; `Get-Service` follows each.

## 6. Install a Development server (private Git and `gh`, forks, paths with spaces)

Do steps 16 and 18 first (the tools and the GitHub sign-in).

30. [ ] **Start the wizard, choose Development**, name it `test dev`, folder `C:\Kora Test\Dev Server`. GitHub step: your
    accounts are listed (not "loading" for ever); pick the test account or organisation. Choose **only a few
    resources** with "Choose everything myself" to keep it short.
31. [ ] **Install.** *Should happen:* Kora uses its own Git and `gh` (nothing says "install git" or asks to restart a
    window), forks are created or reused ("created N forks, reused M"), clones go into
    `C:\Kora Test\Dev Server\resources`. Note any UAC prompt and the step.
    *If not, copy back:* Show details, `install.log`, `kora tools status`.
32. [ ] **Git works inside a folder with spaces.** In a normal PowerShell window run
    `& "<git path from kora tools status>" -C "C:\Kora Test\Dev Server\resources\<a resource folder>" status` (use the
    path under `resources\[kora]\...`): it prints the branch with no error.
33. [ ] **Branches and pull requests.** Open the server > **GitHub** tab > Branches and pull requests.
    - Each resource shows "Normal version (dev)". The Resources tab shows the same.
    - Pick one resource, press **Refresh list**, switch it to another branch of the fork or the original repository.
      The tab and the Resources tab now show "Branch ...". Press **Back to the normal version**.
    - Open a pull request from the list (or paste a number from the original repository). The resource shows "Trying PR #N".
    - Edit a file in that resource's folder (add a line in Notepad), then switch again: Kora must stop and ask. Choose
      **Keep my changes aside**: the switch happens; the changes are listed with **Bring my changes back**; press it and the
      edit is back.
    - Run **Check for updates** on the Resources tab: the switched resource says it is on another branch and is **not**
      updated.
    - Press **Back to the normal version, for all**.
    *If not, copy back:* the Show details text; `kora branch show "test dev" <resource-id>`; a screenshot.
34. [ ] **"Several resources at once".** Type a branch name that exists in only some resources, press **Check which
    resources have it** before anything moves, switch those, then put them all back.

## Kora's administrator service and the MSI (replaces the old UAC / worker checks)

There is no permission prompt any more except the one for the installer itself. Do these on a Windows 10/11 or Server
2019+ machine with a normal administrator account and at least one standard (non-administrator) account. CI
(`windows-smoke.yml`, job `msi`) installs the MSI and drives the service from standard test accounts through the Task
Scheduler; this list is what a real desktop adds (prompts, shortcuts, Apps & features, upgrade over what you have).

Part of this is now checked by CI (`windows-uac.yml`; see [windows-uac-ci.md](windows-uac-ci.md) for which steps and what only a self-hosted VM can do). The list below stays the owner's pass.

Do these on a normal administrator account unless a step says otherwise. The prompt can appear as a flashing shield in
the taskbar instead of on screen: note where it showed.

S1. [ ] **Install the MSI** (double-click `kora-windows-x86_64.msi`, or the one-liner `irm ... | iex` and answer Y).
    *Should happen:* exactly one Windows permission prompt; `C:\Program Files\Kora\kora.exe` exists; Services shows
    "Kora administrator service" running, startup Automatic, log on as Local System; `kora` works in a new terminal (machine
    PATH); a Start Menu "Kora" shortcut (and a Desktop one) opens the panel; Settings > Apps lists Kora with Uninstall.
    *If not, copy back:* what the prompt said (publisher), the MSI log (`msiexec /i ... /l*v log.txt`).
S2. [ ] **Install a Production server** (service + firewall + "set up the database for me") as the account that
    installed Kora. *Should happen:* **no** prompt at all; the server's service, the firewall rule and MariaDB exist; the
    server folder and its files belong to you (not to Administrators or SYSTEM); `kora service status` says "serves this
    account: yes".
S3. [ ] **Standard account.** Sign in as a standard user an administrator added with
    `kora service allow <user>` (run in an administrator terminal). Install a server from that user's panel: no
    password, no prompt; everything is theirs. A second standard account that was **not** added gets "Kora's
    administrator service is not turned on for your account" with the command `kora service allow <account>`.
S4. [ ] **Two accounts, one machine.** As account A install server `alpha`; as account B (also allowed) try to delete,
    change the ports or the schedule of `alpha` (a folder they cannot write is refused by Windows; for a folder B *can*
    write, the service refuses: "does not belong to you"). Settings of B never list A's servers.
S5. [ ] **Nothing runs from a user-writable place.** Delete `%LOCALAPPDATA%\Kora\bin` if it exists, put a renamed
    `notepad.exe` as `kora.exe` there and on your user PATH ahead of Program Files: install a server with a service.
    *Should happen:* no Notepad window; the install works (the service ran `C:\Program Files\Kora\kora.exe` only).
S6. [ ] **Environment cannot steer the service.** Set user variables `KORA_MARIADB_HOME`, `KORA_SELF_UPDATE_API`
    (`http://127.0.0.1:9`), `ProgramFiles`, `TEMP` as in the old W1, sign out and in, install with "set up the database
    for me" and update Kora. *Should happen:* as without the variables; nothing under `C:\Users\Public`. Remove them.
S7. [ ] **Remote session.** From another device (Tailscale or token) install a server: no prompt on the desktop, it
    finishes; the audit log (`C:\Program Files\Kora\data\audit.log`, administrators can read it) has one line per
    operation with your account and no paths.
S8. [ ] **Without the service.** On a second machine use the per-user one-liner (answer **n**). Install with a service
    and firewall: the panel says the service is not installed and offers **Install Kora's administrator service**,
    **Continue without the administrator parts**, **Show me the command**. The first runs the MSI (one prompt, a checksum
    check first) and the install then works without asking again; the second finishes the install without a service.
S9. [ ] **Update through the service.** With an older MSI installed, Settings > Update Kora (or `kora self-update`):
    *Should happen:* no prompt; the service restarts on the new version; `kora --version` and `kora service status`
    show it; `kora.previous.exe` is in `C:\Program Files\Kora\data` until the next update.
S10. [ ] **A broken update rolls back.** (Developer check.) Replace the update source with a program that exits
    immediately: *Should happen:* "The new program did not work ... the previous one was put back"; the service keeps
    running the old version.
S11. [ ] **Upgrade from 0.7.0 / 0.6.x** (see section 7): run the MSI over a machine with the old per-user program, the
    old machine-wide app and existing servers. *Should happen:* servers, services, firewall rules and databases are
    untouched and still manageable from the panel; "Tidy up old shortcuts" sets your own old `kora.exe` aside; an old
    update task that had "Run with highest privileges" no longer has the tick (Task Scheduler > Kora > Update ...).
S12. [ ] **Uninstall** in Settings > Apps: the service, `C:\Program Files\Kora` (including `data` and `owners.json`),
    the PATH entry and the shortcuts are gone; your servers, their services and your saved keys are not touched.
S13. [ ] **Service account narrowing (owner decision).** The service runs as LocalSystem. Review whether a virtual
    account with only the privileges it uses is worth it (service control, firewall, folder ACL changes, task creation
    for other users) and write down what failed if you try one.

## 7. Upgrade delta: a machine that has 0.6.x with the old machine-wide app

Do this **instead of** section 1 on a machine that already has 0.6.x, or on a second VM that has it. To make that
machine: install 0.6.5 (`$env:KORA_VERSION='v0.6.5'` before the install line), then in an **administrator** window
run `kora app install` (this is how 0.6.x made the machine-wide app, with its administrator shortcuts).

35. [ ] **Before upgrading** write down: `kora --version` (0.6.5), the old shortcuts (their Properties show "Run as
    administrator" ticked), `Test-Path "$env:ProgramFiles\Kora\kora.exe"` is `True`, an "Apps & features" entry named
    Kora exists, and the machine `PATH` mentions Kora.
36. [ ] **Upgrade.** In a normal window run `kora self-update` (or the install line). Then `kora app status`:
    it shows the new Start Menu and Desktop shortcuts as present **and** `old_app_present: yes` with items listed.
    `kora --version` says 0.7.0 (in a new window).
37. [ ] **The notice and "Tidy up old shortcuts".** Open the panel: the servers screen shows a notice about old shortcuts;
    Settings > Kora shortcuts offers **Tidy up old shortcuts**. Press it from a normal panel (no helper on Windows):
    *Should happen:* the new shortcuts are made or kept, and the panel shows the command `kora uninstall --app --yes`
    to run as administrator (or, if your build has the elevated worker, a single UAC prompt).
    *If not, copy back:* the Show details text and `kora app status`.
38. [ ] **Run that command in an administrator window.** *Should happen:* it lists and removes the old copy under
    `%ProgramFiles%\Kora`, the machine `PATH` entry, the old shortcuts and the "Apps & features" entry. Then
    `kora app status` says `old_app_present: no` and the new Start Menu and Desktop shortcuts are **still there and still
    work** (double-click one). `kora --version` still works in a new window (the per-user copy in
    `%LOCALAPPDATA%\Kora\bin` is used, and the old copy is gone).
39. [ ] **The background panel survived the clean-up.** `kora` opens the panel; Task Manager shows `kora.exe` running from
    `%LOCALAPPDATA%\Kora\bin`, not from Program Files. If the panel was running from the old copy when you tidied up,
    it should have been moved to the new one (if not: sign out and in and note it).
40. [ ] **Existing servers.** A server made by 0.6.x is listed and starts/stops from the panel. Nothing about it changed,
    including its folder (it was not moved).
41. [ ] **`kora install --dry-run --name x --db skip`** (with `CFX_LICENSE_KEY` set to any text) shows the folder
    `C:\Kora\x`, not `%USERPROFILE%\kora\x`.

## 8. Uninstall

42. [ ] **`kora app uninstall --dry-run`** (normal window): lists the Start Menu and Desktop shortcuts as the only
    things it would remove and changes nothing. Run `kora app uninstall --yes`: both shortcuts disappear; the program,
    the servers and the panel are untouched; **no UAC prompt**. Make them again with `kora app install`.
43. [ ] **Panel Uninstall.** Settings > Uninstall Kora shows an item-by-item list that names the shortcuts and Kora's own
    Git/GitHub CLI copies. Do **not** tick the server folders or databases. Run it (type `uninstall`): services, firewall
    rules and the server-list entries go; **the server folders and `secrets.json` stay**.
44. [ ] **`kora delete "test dev"`** and **`kora delete "test prod"`** (administrator window if it says so) remove the
    server, its service and its firewall rule: `Get-Service kora-*` and
    `Get-NetFirewallRule -DisplayName "*kora*"` show nothing for them.
45. [ ] **Tidy the test machine**: delete the test forks on GitHub, run `kora uninstall` in a terminal and answer **yes**
    to removing saved keys and Kora's own tools; the folder `%LOCALAPPDATA%\Kora` is gone or empty apart from the program.

## 9. Report back

Paste into the release issue: the Kora version, Windows version (`winver`), whether each numbered step passed, the UAC
prompts you saw (which step, how many), and for each failure the items listed in section 0.
