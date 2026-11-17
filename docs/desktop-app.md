# The Kora desktop app (tray icon and window)

An optional program, `kora-desktop` (`kora-desktop.exe` on Windows), that gives Kora a place on the desktop:

* an icon in the notification area (Windows) or system tray (Linux) that shows at a glance whether your servers
  are running, with a menu to open Kora, start, stop and restart a server, check for updates, pause notifications
  and turn "Start with Windows" on or off;
* a small native window that shows the same panel you get in a browser (it is the web UI, not a rewrite);
* desktop notifications when a server stops unexpectedly or an update is ready, that open the right page when
  clicked;
* one copy at a time (starting it again shows the window), and closing the window keeps it in the tray.

It is a **thin client** of the panel. The panel is the same `kora panel` background process as before, on loopback
only. The browser route is unchanged: remote use, headless servers and people who prefer a browser keep working, and
nothing on a headless Linux server changes. The `kora` program and the panel never depend on the app.

Contents: [Using it](#using-it) · [How it works](#how-it-works) · [Why a separate crate](#why-a-separate-crate) ·
[Choice of toolkit](#choice-of-toolkit) · [Threat model](#threat-model) · [Administrator prompts in front](#administrator-prompts-in-front) ·
[Autostart, shortcuts, uninstall](#autostart-shortcuts-uninstall) · [Shipping it in the MSI and the .deb](#shipping-it-in-the-msi-and-the-deb) ·
[CI](#ci) · [What has never run](#what-has-never-run-assumptions-to-check-first)

## Using it

Start `kora-desktop` (or the Kora shortcut, which `kora app install` points at it when it sits next to `kora`).

| Tray icon | Meaning |
|---|---|
| green | every server runs and nothing needs doing |
| amber | something is worth a look: a server stopped on purpose, a warning, an update is ready |
| red | something needs attention: a server stopped unexpectedly, a server folder is missing, a danger alert |
| grey | Kora's panel is starting or not reachable, or there are no servers yet |

* **Left click** (Windows) opens the window; **right click** shows the menu. On Linux only the menu exists (the
  AppIndicator protocol has no click on the icon).
* **Quit** closes the icon and the window. **Your servers and Kora's background service keep running.** The menu says
  so. **Quit and turn off Kora's background service** also removes the background panel (`kora panel
  --remove-service`); the game servers still keep running.
* **Closing the window** keeps Kora in the tray; the first time it says so in a notification.
* **Pause notifications** (1 hour, 8 hours, until tomorrow) silences the app's notifications. It does not change the
  alert rules in the panel (Monitoring and Notifications tabs): a rule that is switched off there never produces a
  notification here either.
* **Start with Windows / when I sign in** is on after the first run and is yours to switch off.
* Command line: `kora-desktop [--background] [--open <page>] [--kora <path>]`. `--background` starts in the tray with no
  window (what autostart uses), `--open /settings` shows that page (a second launch hands it to the first),
  `--uninstall` removes the start-up entry and notification identity.

If the window cannot be made (no WebView2 on Windows, no WebKitGTK on Linux) the app says so in one notification and
opens Kora in the browser instead. If a Linux desktop has no tray (some GNOME setups), the app runs with a normal window
and closing it quits.

## How it works

```
kora-desktop ──runs──▶ kora desktop session   (hidden command: the same find-or-start logic as a bare `kora`)
     │                    prints ONE JSON line: port, one-time sign-in link, token
     │
     ├── window: wry (WebView2 / WebKitGTK) loads http://127.0.0.1:<port>/#code=…   (signed in, loopback only)
     ├── thread: GET /api/v1/events (SSE) + /alerts + /self-update, Bearer token ──▶ tray state and notifications
     └── tray menu actions: POST /api/v1/servers/{s}/control, open links in the default browser
```

* **Starting the panel.** The app does not copy the logic that finds or starts the panel. It runs `kora desktop session`
  (`crates/kora-cli/src/cmd/desktop.rs`), which uses `bare::facts` + `bare::decide` + `bare::start_and_wait`: the panel
  already runs → its port and a one-time link; otherwise the background panel is set up and started (as for a bare
  `kora`); where no background service is possible (`foreground_needed`) the app runs `kora panel --foreground --no-open`
  itself; from an administrator session it says the same sentence as `kora` does. It never asks for administrator
  rights.
* **Signed in.** The window loads the one-time `#code=` link (it works once, for a few minutes). A later window gets a
  fresh code from `POST /session/link`. The session lives in the window's own cookie store
  (`%LOCALAPPDATA%\Kora\desktop\webview`, `~/.local/state/kora/desktop/webview`), separate from every browser.
* **Status.** The tray reads the existing event stream (`servers` events carry the whole list) and polls `/alerts`
  (every 8 s) and `/self-update` (20 s after the start, then every 6 hours). Nothing new was added to the panel's API.
  If the stream ends (the panel restarted, a new port) the app asks `kora desktop session` again, and reloads the
  window with a new code.
* **Notifications** (`crates/kora-shell/src/toast.rs`): the panel's monitor already raises alerts according to the
  person's per-server settings, so the app only turns NEW alerts into notifications: each alert once; no replay of old
  alerts at start (except a fresh danger); a burst becomes one summary; pause; optional quiet hours (`quiet_start`,
  `quiet_end` in minutes after midnight in `shell.json`, only dangers get through). The page can also ask for a
  notification (`window.koraShell.notify`) for things only it knows, and `AlertsProvider` does that for every
  success/warning/danger banner **when the window is not in front** (install finished, update failed, ...). Clicking a
  notification opens its page (`/servers/<name>`, `/settings`).
* **Single instance** (`instance.rs`): an exclusive file lock plus a loopback socket with a random nonce; a second
  launch sends `focus [route]` and exits.
* **State.** The app's own preferences (first-time hint shown, pause, window size, quiet hours) are in
  `shell.json` in its state folder, *not* in the Kora store: they belong to this device's app, not to Kora's servers or
  accounts, and the panel owns the store (the app talks to the panel only through its API). A missing or damaged file
  means defaults.
* **Browse buttons.** Inside the app the web UI shows a **Browse...** button next to the "Server folder" box
  (`web/src/lib/shell.ts`: `window.koraShell.pickFolder/pickFile`, native dialogs). In a browser nothing changes. The
  folder/file chosen is returned to the page, which then uses the panel's ordinary operations. "Open folder" does
  not need the app: the panel already has `POST /servers/{s}/open-folder`, and it works for the person running the app.

## Why a separate crate

`crates/kora-shell` (in the main workspace, **no GUI dependency**: only `serde`, `serde_json`) holds every decision as
pure functions and is tested by the normal CI (`cargo test --workspace`) on Linux and Windows: tray state and menu model,
notification coalescing, deep-link routing, the single-instance protocol, autostart file contents, the navigation
allow-list, the bridge's allow-list, the SSE parser, the session output parser, the icons (drawn in code).

`crates/kora-desktop` (the GUI layer: window, tray, toasts, dialogs, elevation) is **its own Cargo workspace with its
own `Cargo.lock`** and is excluded from the main one (`exclude = ["crates/kora-desktop"]`). Reasons:

* `cargo build/test/clippy --workspace`, `cargo deny`, and the Linux/Windows/e2e CI jobs never compile or resolve a GUI
  toolkit (about 300 packages across both systems, and system libraries on Linux: WebKitGTK, GTK 3). The main build
  stays fast and dependency-light.
* It needs system packages on Linux that the main CI image and the people who build `kora` do not have.
* The supply-chain check for it is its own job (`.github/workflows/desktop.yml`, `crates/kora-desktop/deny.toml`) with
  its own, documented exceptions, so GTK 3's unmaintained bindings cannot weaken the rule for the main program.

The cost: one more lockfile and one more workflow, and the GUI layer is not part of `cargo test --workspace`.

## Choice of toolkit

Compared: **Tauri v2** and **`wry` + `tao` + `tray-icon` + `notify-rust` directly** (the pieces Tauri is built from).
Versions and facts below were read from crates.io and each project's own README/source on **2026-11-17**.

| | direct pieces (chosen) | Tauri v2 (2.12.6) |
|---|---|---|
| window | `wry` 0.57.2 (WebView2; WebKitGTK 2.40+ via `webkit2gtk-4.1`) | the same `wry` and `tao` underneath |
| tray | `tray-icon` 0.26 (+ `muda` for menus; Linux: libappindicator or StatusNotifier) | the same |
| notifications | `notify-rust` 4.19: Windows toasts with click callbacks, freedesktop notifications with actions | notification plugin = `notify-rust` |
| single instance, autostart | about 150 lines of std code (`instance.rs`, `autostart.rs`), tested here | two plugins |
| file dialogs | `rfd` 0.18 | dialog plugin = `rfd` |
| extra | none | a build step (`tauri-build`, codegen of capabilities, config, ACLs), a JS IPC layer and permission system we would then have to turn OFF to keep "no bridge beyond a tiny allow-list", a larger tree and more to audit |
| licences | all MIT or Apache-2.0 (checked for every package of the lockfile, see `deny.toml`) | the same |

Tauri's strength is bundling (installers, updater) which Kora already has (MSI, `.deb`, its own checksum-verified
self-update) and its JS bridge, which we do not want. The direct pieces do exactly what is needed with no unused
machinery, and the bridge is ours: one JSON message type with an allow-list of six commands (`kora-shell/src/bridge.rs`).
Maintenance: all of `wry`, `tao`, `tray-icon`, `muda` are Tauri-maintained and updated in lockstep with Tauri (wry 0.57.2
was published 2026-11-10); `rfd` and `notify-rust` are widely used and were updated in November 2026.

Size: the release profile is `opt-level = "s"`, LTO, stripped. The HTTP client is `ureq` **without** TLS (loopback only),
so no `rustls`/`ring` enters the app. Not measured on this machine (nothing GUI is built here): CI prints the file size
in the artifact listing; a few MB is expected on Windows (WebView2 is not bundled; it is part of Windows 10/11).

Known costs of the Linux build: `wry` and `tray-icon` use the **GTK 3** bindings (`gtk` 0.18 family), which their authors no
longer maintain and which carry RustSec "unmaintained" advisories (and one `glib` unsoundness advisory); they are
listed with reasons in `crates/kora-desktop/deny.toml` and apply to the Linux build only. The Windows build does not
contain them. This is the main reason the Linux app is "second".

## Threat model

What an attacker could want: run code or read files through the window, make Kora act (start/stop servers, install
something, read licence keys) through a page that is not the panel, or gain administrator rights through the app.

* **The window only ever shows the panel's origin** (`http://127.0.0.1:<its port>`). `kora-shell/src/nav.rs` decides
  every navigation and every "open in new window": the panel's origin is allowed; any other `http(s)` address opens in
  the default browser and the window stays; every other scheme (`file:`, `javascript:`, `data:`, custom handlers) is
  refused; `localhost`, `[::1]`, other ports, user-info tricks (`http://127.0.0.1:47120@evil.example`), backslashes and
  control characters are not the panel. It is a plain-string parser with a strict "anything unclear is foreign" rule, with
  tests for each trick.
* **No remote content gets the app's privileges.** `window.koraShell` is injected into the main frame only, and every
  message is checked against the sender's address (`trusted_page`) before it is parsed.
* **The bridge is an allow-list** (`bridge.rs`): `hello`, `pick_file`, `pick_folder`, `open_external` (http/https only),
  `notify` (text, and a panel *route*, never an address), `elevate` (one fixed action by name). No way to run a program,
  read or write a file, or pass a path to the shell. Text is length-limited; unknown or malformed requests are refused
  with a plain message. A picker returns a path TO the page only.
* **Credentials.** The one-time sign-in code (works once, minutes) goes in the window's address. The panel token
  (needed for the tray's API calls) travels only over the standard output of the `kora` child the app started, is kept in
  memory, goes to the panel only in an `Authorization` header over loopback, and is never logged, written to a file, put
  in a command line or shown by `Debug` (`Session` redacts it). The panel's existing checks apply to the app like to any
  client: the token only works from loopback, state-changing requests need it.
* **Navigation inside the page is the panel's own.** The panel's content-security rules are unchanged. Downloads from the
  panel (backups) use the platform default, as in a browser.
* **Single instance socket.** Loopback, a random nonce in a file only the user can read, one command (`focus`) whose only
  argument is a panel route. The worst a forged line can do is raise a window.
* **Notifications** show alert titles and messages the panel wrote (server names, sizes). No secrets are in alerts
  (the panel's own rule); the app adds none. A notification can only open a panel route.
* **Administrator rights.** The app never runs elevated and refuses to start the panel elevated (that is `kora`'s rule).
  The page can ask for exactly one elevated action by name; the shell maps it to a fixed command line
  (`platform::elevated_command`), the page supplies no program, argument or path. Windows' own prompt still has to be
  confirmed by the person.
* **Supply chain.** The app adds the GUI toolkit to the product, but only to the separate binary; the main program's
  dependency tree gained only the `kora-shell` crate (no new external crates). The toolkit is built from pinned,
  locked versions in CI with its own `cargo deny` job.
* **Not defended:** malware already running as the same user (it can read the token file and the shell's files anyway,
  as it can today with the panel's token file).

## Administrator prompts in front

What decides where Windows' permission prompt (UAC) appears: the prompt is shown by the system on the secure desktop,
but whether it is brought **in front** or only flashes in the taskbar depends on whether the process that asked has
the right to bring a window forward. A background process (the panel is started at sign-in with no window) does not;
`ShellExecuteEx` with the verb `runas` from it ends up as a flashing shield. A foreground app that passes its own
window as the owner (`SHELLEXECUTEINFO.hwnd`) and is the foreground window makes the prompt belong to that window.

So the shell is the launcher for the one remaining prompt, "Install Kora's administrator service" (it launches the
MSI): the web UI asks `window.koraShell.elevate('install_service')` (`elevateInstallService()` in
`web/src/lib/shell.ts`, returns false outside the app so the existing path is used); the app shows and focuses its
window, calls `SetForegroundWindow`, then `ShellExecuteExW("runas", <fixed command>, hwnd)` with `SEE_MASK_NOASYNC`
(`os.rs::elevate`). Declining the prompt gives "Windows asked for permission and it was not given." (error 1223).

**Seam for the privileged-service work:** the fixed command is `kora service install`, in one place
(`kora-shell/src/platform.rs::elevated_command`). If its name or arguments differ at merge, change it there. The web
button that offers "Install Kora's administrator service" should call `elevateInstallService()` first and fall back to
what it does today when that returns false. Nothing in the app edits or depends on the service code.

## Autostart, shortcuts, uninstall

* **Autostart** is per user, no administrator rights. Windows: the value `Kora` under
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` = `"<path>\kora-desktop.exe" --background`, written with
  `reg.exe`; Windows' Settings/Task Manager can switch it off without deleting it (`StartupApproved\Run`), which the
  menu respects. Linux: `~/.config/autostart/kora-desktop.desktop` (marked `X-Kora-Desktop=1`; only files with that mark
  are ever removed). The first run turns it on once (`autostart_default_applied` in `shell.json`); the tray toggle is the
  person's. It replaces the *role* of the Startup-folder script of the background panel for desktop users: the app
  starts in the tray and makes sure the panel is up. The panel's own Startup script / systemd unit is untouched, so
  nothing breaks for people without the app.
* **Shortcuts.** `kora app install` makes the Start Menu / Desktop shortcut (Windows) or the applications-menu launcher
  (Linux) point at `kora-desktop` when it sits next to `kora` (`app_install::user::launcher_exe`), else at a bare `kora`
  exactly as before. Those shortcuts are still recognised as Kora's own, so `kora app uninstall` removes them.
* **Uninstall.** `kora app uninstall` also runs `kora-desktop --uninstall` (removes the Run value / autostart file and the
  notification identity). The app's state folder (`shell.json`, the window's cookies) is left; deleting the folder
  removes it.
* **Names.** The program is `kora-desktop(.exe)`, not `Kora.exe`: on Windows that would be the same file name as `kora.exe`
  (the file system ignores case).

## Shipping it in the MSI and the .deb

The MSI, the `.deb` and `release.yml` belong to the privileged-service work. Nothing there was edited; these are the pieces
to add at merge. **Decide where the app's binary comes from:** `desktop.yml` builds it and uploads
`kora-desktop-<os>-x86_64` with `SHA256SUMS.txt`; `release.yml` should download that artifact (or build it the same way)
and sign it with the same signing step as `kora.exe` (an unsigned program that starts at sign-in and shows
notifications is what SmartScreen and antivirus distrust most).

### Windows MSI (WiX v4 fragment; adapt the ids to the existing product)

```xml
<!-- next to kora.exe, in the same install folder (ProgramFiles\Kora) -->
<Component Id="KoraDesktopExe" Directory="INSTALLFOLDER" Guid="*">
  <File Id="KoraDesktopExe" Source="$(var.DesktopDir)\kora-desktop.exe" KeyPath="yes" />
</Component>

<!-- The Start Menu shortcut points at the app. AppUserModel.ID is the notification identity ("Kora.Desktop"):
     with it Windows shows toasts as "Kora" with no registry help (the app also registers it per user, harmlessly). -->
<Component Id="KoraDesktopShortcut" Directory="ProgramMenuFolder" Guid="*">
  <Shortcut Id="KoraDesktopStartMenu" Name="Kora" Description="Your FiveM servers at a glance"
            Target="[INSTALLFOLDER]kora-desktop.exe" WorkingDirectory="INSTALLFOLDER" Icon="KoraIcon">
    <ShortcutProperty Key="System.AppUserModel.ID" Value="Kora.Desktop" />
  </Shortcut>
  <RegistryValue Root="HKCU" Key="Software\Kora\Installer" Name="DesktopShortcut" Type="integer" Value="1" KeyPath="yes" />
  <RemoveFolder Id="RemoveProgramMenu" On="uninstall" />
</Component>

<!-- On uninstall, take the app's start-up entry out for the user who runs the uninstall (it is per user; other
     users' entries point at a missing file and do nothing). -->
<CustomAction Id="KoraDesktopUninstall" Directory="INSTALLFOLDER" ExeCommand="[INSTALLFOLDER]kora-desktop.exe --uninstall"
              Execute="deferred" Impersonate="yes" Return="ignore" />
<InstallExecuteSequence>
  <Custom Action="KoraDesktopUninstall" Before="RemoveFiles" Condition="REMOVE=&quot;ALL&quot;" />
</InstallExecuteSequence>
```

* **No autostart from the MSI.** The app turns it on per user at first run (so it is the person's, can be switched off in the
  tray, and an elevated installer does not write another user's registry). If a machine-wide default is wanted instead,
  write `HKLM\...\Run\Kora` from the MSI and remove the first-run default; the tray toggle then only controls the per-user
  value.
* **WebView2 decision: do not bundle the runtime in the MSI.** It is preinstalled on Windows 11 and on Windows 10 since
  the Edge update of 2022, updates itself, and adding the 100+ MB offline installer (or a download at install time) would
  make the MSI large and slow for the many people who already have it. The app detects its absence (`wry::webview_version`),
  opens Kora in the browser instead and says in one notification where to get it
  (`https://go.microsoft.com/fwlink/p/?LinkId=2124703`, Microsoft's small "Evergreen Bootstrapper", verified to redirect to
  `MicrosoftEdgeWebview2Setup.exe`, 2026-11-17). If an organisation needs it guaranteed, chain the bootstrapper in a WiX
  Burn bundle with `MicrosoftEdgeWebview2Setup.exe /silent /install` and a detect condition on
  `HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}` (`pv`).
* After install the MSI may launch the app for the installing user (`kora-desktop.exe`, not elevated: use a
  non-elevated launch such as a `Launch` entry that runs as the user in the finish dialog; never from the elevated
  deferred sequence, where it would run as administrator and show the prompt behind).

### Linux `.deb`

```
usr/bin/kora-desktop                      (mode 0755)
usr/share/applications/kora-desktop.desktop

Package: kora   (or a second package `kora-desktop`, recommended: servers without a screen do not need the GTK libraries)
Depends:    libwebkit2gtk-4.1-0, libgtk-3-0
Recommends: libayatana-appindicator3-1
Suggests:   xdg-desktop-portal     (the Browse dialogs)
```

```ini
# /usr/share/applications/kora-desktop.desktop
[Desktop Entry]
Type=Application
Name=Kora
Comment=Your FiveM servers in a window, with a tray icon
Exec=/usr/bin/kora-desktop
Icon=kora
Terminal=false
Categories=Network;Utility;
```

* Do **not** install a system-wide `/etc/xdg/autostart` entry: the app writes the per-user one on first run (and removes it
  with `--uninstall`). A `postrm` that runs `kora-desktop --uninstall` is not needed: a user's autostart file that points to
  a missing program is ignored by desktops (some show a "failed to start" once; if that matters, `postrm purge` can
  remove `/home/*/.config/autostart/kora-desktop.desktop` files that contain `X-Kora-Desktop=1`).
* The package must not make the `kora` package depend on GTK/WebKit: a split package keeps headless servers free of it.

## CI

`.github/workflows/desktop.yml` (path-filtered to the app and `kora-shell`): a `cargo deny` job for the app's own
lockfile; then on `windows-latest` and `ubuntu-latest`: build `kora` and the app (release, `--locked`), clippy for the app,
the smoke test, checksums, artifact. System packages for Linux are installed in the job. The smoke test
(`scripts/smoke/desktop-smoke.sh` under xvfb with a session bus; `desktop-smoke.ps1` on the Windows runner's desktop
session) starts the app with `--smoke <report>` against a throw-away `KORA_HOME`/`KORA_DESKTOP_HOME`, with no background
service (`kora desktop session --no-background`) and no start-up change, and asserts: the window title is `Kora`, the
page title mentions Kora, the window shows `http://127.0.0.1:<port>/`, the event stream is connected, the tray **menu
model** has Open/Quit/Quit-and-stop, one driven action (Open on `/settings`) navigates the window there, a second launch
(`--open /system`) reaches the first and exits 0, then the app quits by itself. On Windows it also asserts the tray icon
was created; under xvfb a tray host is normally missing, so only the menu model is asserted there.
The `kora-shell` crate's unit tests run in the main `ci.yml` (it is a workspace member).

## What has never run (assumptions to check first)

Only the pure logic (`kora-shell`, the `kora desktop session` command, the shortcut change, the web side) was run and
tested before this was written. **The GUI layer (`crates/kora-desktop`) has never been compiled or run**: it was written
against the sources of wry 0.57.2, tao 0.37.2, tray-icon 0.26, muda 0.21, notify-rust 4.19, rfd 0.18 and windows-sys 0.61,
and its first CI runs will need fixes. Assumptions, in the order to suspect them when something does not work:

1. **WebView2 `NavigationStarting` and the navigation handler.** The first load (`with_url`) and the SPA's own navigation
   are expected to report `http://127.0.0.1:<port>/...`. `about:blank` is allowed. If the window stays blank, the handler
   probably refused a URL it was given (add an `eprintln!` of `u` in `create_window`).
2. **`window.ipc.postMessage` on Windows/Linux** exists in wry 0.57 and the handler's `req.uri()` is the page's address
   (documented as the main-frame address on Linux). If `trusted` is false for a legitimate message, check the uri.
3. **Initialization script runs before the page's scripts** (`with_initialization_script_for_main_only(.., true)`).
4. **Toasts need an identity.** `notify-rust` on Windows uses `tauri-winrt-notification` with app id `Kora.Desktop`; we
   register that name under `HKCU\Software\Classes\AppUserModelId` at start (`platform::aumid_register_args`). If no toast
   appears, Windows dropped it for an unregistered identity: switch the id to
   `Toast::POWERSHELL_APP_ID` (shows as "Windows PowerShell", but always works) in `toasts.rs`.
5. **Toast click** is reported as `NotificationResponse::Default` on Windows and via the `"default"` action on Linux. A
   click on a toast after the app was closed does nothing. The thread waiting for a click stays until the toast is
   dismissed or clicked (a few threads at most because notifications are coalesced).
6. **Tray.** Windows: left click `Click { Left, Up }` opens the window; the menu is rebuilt only when the model changes.
   Linux uses `libappindicator` (default feature of `tray-icon`): needs `libayatana-appindicator3` at run time and a
   desktop that shows indicators (GNOME needs the AppIndicator extension). Without one `TrayIconBuilder::build` may succeed
   without any visible icon; then closing the window only hides it, and the launcher/`--open` brings it back.
7. **Elevation.** `ShellExecuteExW` with `runas` and the window as owner is called ON the UI thread and blocks while the
   prompt is up; `SetForegroundWindow` may be refused by Windows if the app is not the foreground process (then the prompt
   still opens, owned by the window). Declined prompt = error 1223. Whether the prompt really appears in front on every
   Windows version is exactly what the checklist section asks the owner to confirm.
8. **File dialogs** run on a helper thread without a parent window (they may open behind the window on some systems).
   On Linux they use the XDG desktop portal (no portal = no dialog; the page then keeps the typed box).
9. **Elevated CI runner.** The Windows runner is an elevated administrator; `KORA_ALLOW_ROOT_PANEL=1` is set so `kora`
   starts the panel. A real run as a normal user takes the ordinary path.
10. **`reg.exe` output.** `autostart::run_value_from_reg_query` and `approved_from_reg_query` read what `reg query` prints
    (`    Kora    REG_SZ    <value>`; `REG_BINARY    0200...` for StartupApproved). If the toggle shows the wrong state,
    compare with the real output.
11. **The window icon** is the generated green badge; there is no `.ico` resource in the exe yet (Explorer shows the
    default program icon until one is embedded with a build script).
12. `cargo deny` for the app has never run: `deny.toml` was written from the licences of the lockfile (checked against
    crates.io) and the RustSec ids of the GTK 3 bindings by name; expect to adjust the exceptions on the first run.
