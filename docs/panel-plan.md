# Kora Panel plan (0.4.0)

Turn the installer into a local web panel that installs, manages and monitors FiveM servers on
Windows and Linux. The CLI and the TUI stay. Hard requirement: **parity** — nothing the CLI, the
wizard or the manager does today may be lost, with one recorded exception (setup codes, below).

## 1. Decisions (settled)

| Topic | Decision |
|---|---|
| Privileges | The panel runs as the installing user. Privileged actions go through a small root helper (installed once, Unix socket / named pipe, peer-credential check, accepts only the existing privileged verbs). Without the helper the panel shows the exact `kora ...` command to run. |
| Lifetime | `kora panel` runs in the foreground; an opt-in service keeps it running for monitoring, alerts and remote access. |
| Server | axum + tokio, confined to `crates/kora-web`. `kora-core` stays sync; every core call runs in `spawn_blocking`. |
| Frontend | Vite + React + TypeScript + Tailwind + shadcn/ui in `web/`, built to static files and embedded in the binary. No Next.js. Built in CI (`npm ci --ignore-scripts`, committed lockfile); built assets are not committed. `cargo build` without Node still works and serves a "UI not built" page. |
| Remote destructive actions | Delete, uninstall and console input are local-only by default, enabled per node; always require typing the server name. Roles: viewer / operator / admin. |
| Remote identity | tailscaled whois on the peer address; identity headers are never trusted. Loopback requests always need the token. A token/password provider covers Headscale, WireGuard and tunnels. |
| Setup codes | Removed in 0.4.0 (the one parity exception). `--code` stays parseable and fails with a clear message. `--hostname` and `--game-build` are added so headless installs lose nothing. The website's code flow is removed after 0.4.0 is on GitHub. |
| Hub | Any panel can act as a hub. Nodes advertise version and supported operations; no central service. |
| Monitoring | In-memory ring plus a capped file per server under `.kora/`, about 24 hours. Alerts: in-panel banner, Discord webhook, generic webhook, email (email last). |
| Headless | `kora panel --no-open`, auto-detected when there is no display or in an SSH session; prints the URL, the token and an `ssh -L` hint. |
| macOS | Not supported. Mac-specific code and wording are removed. |
| Signing | The Windows exe stays unsigned. No SignPath. Docs say so and explain `.sha256` verification. |

## 2. Architecture

```
kora-cli ──┬── kora-tui ──┐
           └── kora-web ──┼── kora_core::api  (Backend trait, Op enum, DTOs, jobs)
web/ (React) ── HTTP ─────┘        └── the rest of kora-core (unchanged engine)
```

- `kora_core::api` is the only way front ends reach the engine. The TUI, the CLI and the web
  server use the same trait, so they cannot drift.
- `RemoteBackend` (later) implements the same trait over HTTP for the hub.
- Logic that lives in `kora-tui` today and is not about drawing moves to core: `manager/backend.rs`,
  `manager/adapter.rs` (`SystemBackend`, `with_ensure`, `valid_resource_name`, `read_badge`),
  `manager/adopt.rs`, the file parts of `manager/logs.rs` and `manager/console.rs`.

### 2.1 Operations

One list defines every operation: `kora_core::api::Op` with `Op::ALL`, a stable `name()`
(snake_case, used in `/meta` capabilities) and a `route()` (method + path). The web router is an
exhaustive `match` over `Op` (no wildcard arm), so an operation without a handler does not compile.

All routes are under `/api/v1`. `{s}` is the server's registry name, percent-encoded.

| Op | Route | Notes |
|---|---|---|
| meta | GET /meta | `{version, api: 1, platform, ops: [names], node}`; no auth needed |
| session_create | POST /session | body `{token}` → sets the session cookie |
| system_info | GET /system | admin?, platform, helper installed?, headless?, kora version |
| servers_list | GET /servers | entries + status + badge + managed |
| server_get | GET /servers/{s} | entry, status detail (running, game_listening, txadmin_up), badge |
| server_control | POST /servers/{s}/control | `{action: start|stop|restart}`; job. Falls back to detached launch when there is no service (same as the CLI). |
| server_delete | DELETE /servers/{s} | `{confirm_name, keep_files, keep_db}`; job |
| server_unregister | DELETE /servers/{s}/registration | adopted servers |
| servers_scan | POST /servers/scan | adopt candidates; job |
| servers_adopt | POST /servers/adopt | `{dir}` |
| server_open_folder | POST /servers/{s}/open-folder | local sessions only |
| logs_list | GET /servers/{s}/logs | log sources |
| logs_tail | GET /servers/{s}/logs/{file} | `?lines=` |
| console_stream | GET /servers/{s}/console/stream | SSE |
| console_send | POST /servers/{s}/console | `{command}` |
| console_enable | PUT /servers/{s}/console/enabled | installs `kora_console` |
| resources_list | GET /servers/{s}/resources | |
| resource_ensure | POST /servers/{s}/ensure | `{name, restart}` |
| update_preview | POST /servers/{s}/update/preview | `{scope}`; job |
| update_apply | POST /servers/{s}/update/apply | `{scope, restart, skip}`; job with events |
| update_auto | POST /servers/{s}/update/auto | job |
| snapshots_list | GET /servers/{s}/snapshots | |
| snapshot_rollback | POST /servers/{s}/snapshots/{id}/rollback | job |
| catalog_for_server | GET /servers/{s}/catalog | catalog, installed ids, note |
| resources_plan | POST /servers/{s}/resources/plan | `{ids}` |
| resources_add | POST /servers/{s}/resources | `{ids}`; job |
| webhook_get | GET /servers/{s}/webhook | redacted |
| webhook_set | PUT /servers/{s}/webhook | `{url}` |
| webhook_remove | DELETE /servers/{s}/webhook | |
| webhook_test | POST /servers/{s}/webhook/test | |
| schedule_get | GET /servers/{s}/schedule | |
| schedule_set | PUT /servers/{s}/schedule | privileged |
| schedule_remove | DELETE /servers/{s}/schedule | privileged |
| ports_set | PUT /servers/{s}/ports | stopped servers only |
| issue_report | POST /servers/{s}/issues | `{resource, title, text}` |
| pr_plan | POST /servers/{s}/pr/plan | `{resource}` |
| pr_open | POST /servers/{s}/pr | `{plan, title, text}` |
| github_status | GET /github/status | |
| github_login | POST /github/login | device flow: `{code, url}`; job completes on approval |
| github_owners | GET /github/owners | |
| catalog_get | GET /catalog | live catalog, bundled fallback |
| artifacts_list | GET /artifacts | |
| install_defaults | GET /installs/defaults | suggested name/dir/ports, saved answers |
| install_saved_clear | DELETE /installs/saved | |
| check_ports | POST /checks/ports | with next-free suggestions |
| check_disk | POST /checks/disk | |
| check_db_login | POST /checks/db-login | |
| install_plan | POST /installs/plan | dry run |
| install_start | POST /installs | body = `InstallRequest`; job with `ProgressEvent`s |
| install_log | GET /installs/{job}/log | |
| self_update_check | GET /self-update | |
| self_update_apply | POST /self-update | `{version?}`; job |
| app_status | GET /app | |
| app_install | PUT /app | `{desktop_shortcut, force, dry_run}`; privileged |
| app_uninstall | DELETE /app | `{dry_run}`; privileged |
| uninstall_plan | POST /uninstall/plan | items that would be removed |
| uninstall_run | POST /uninstall | `{answers: [{item, remove}], delete_files, delete_databases, only}`; job |
| jobs_get | GET /jobs/{id} | |
| job_events | GET /jobs/{id}/events | SSE |
| events | GET /events | SSE: server status / badge changes |
| metrics_get | GET /servers/{s}/metrics | `?range=1h|6h|24h` (default 1h) → `{samples: [...], latest}`; a sample is `{t, up, players?, max_players?, cpu?, ram_mb?, sys_ram_pct?, disk_free_mb?, disk_free_pct?, game_up, txadmin_up, badge?}` (absent = not measurable). Thinned to at most 360 points. |
| alerts_list | GET /alerts | `{alerts: [{id, server, kind, tone, title, message, raised, resolved, acked}]}` newest first; kinds `stopped`, `recovered`, `disk_low`, `ram_high`, `update_attention`; the UI polls it |
| alerts_ack | POST /alerts/{id}/ack | 204; the alert stays listed |
| alert_settings_get | GET /servers/{s}/alerts | `{enabled, rules{stopped,recovered,disk,ram,updates}, disk_min_gb, disk_min_percent, ram_max_percent, discord, webhook{configured, redacted}, email{supported:false}}`; the webhook address is never returned |
| alert_settings_set | PUT /servers/{s}/alerts | partial update of the same fields plus `webhook_url` (`""`/null removes it) |
| nodes_list | GET /nodes | hub: nodes with reachability, version, api, server count, last error; `token_set` only, never the token. **Admin, local only.** |
| node_add | POST /nodes | hub: `{name, url, token}`; checks `/meta` and the token (server list) before saving; `insecure` flag for plain http to a non-private, non-Tailscale address. **Admin, local only.** |
| node_remove | DELETE /nodes/{id} | hub. **Admin, local only.** |
| node_servers | GET /nodes/{id}/servers | hub: a node's servers with the `actions` it supports. Operator or above (read: viewer). |
| node_server_control | POST /nodes/{id}/servers/{s}/control | hub: `{action}`; job proxied to the node's own job (events relayed, result as local); refused with `node_unsupported` when the node's `/meta` lacks `server_control`. Operator or above. |
| fleet | GET /fleet | hub: this panel's servers plus every node's, fetched in parallel; partial when a node is down (`partial`, per-node `last_error`). Viewer or above. |
| remote_get | GET /remote | remote access settings (no token hashes); local sessions only |
| remote_set | PUT /remote | `{enabled?, tailscale?, tailscale_users?, allowed_hosts?, allow_remote_destructive?}`; local sessions only |
| users_list | GET /users | `{users: [{name, role, created}]}`; local sessions only |
| user_add | POST /users | `{name, role}` → 201 `{name, role, token}`; the token is shown once; local sessions only |
| user_remove | DELETE /users/{name} | 204; their sessions end at once; local sessions only |
| session_logout | POST /session/logout | ends this session, clears the cookie; 204 |
| assets_list | GET /servers/{s}/assets | paid (Cfx.re Portal) assets: `{assets: [{name, path, version, imported_unix, present, has_fxap, backups}], hints: [{kind, resource, line, fix}], portal_url}`; hints come from the newest console output ("lack the required entitlement", "failed to verify protected resource", "could not be downloaded"). Operator or above. |
| asset_inspect | POST /servers/{s}/assets/inspect | body = the zip (`application/zip`, name in `X-File-Name`, URL-encoded), streamed to `<server>/.kora/asset-uploads/<id>/`, up to 2 GiB → `{plan, upload}` (`upload` is null when the plan is refused); or JSON `{download: "<file>.zip"}` for a recent zip in the Downloads folder (local sessions only) → `{plan, download}`. Admin. |
| asset_import | POST /servers/{s}/assets | `{upload: id}` or `{download: name}`, `replace` (default true); job with `status` events; result `{zip_name, imported: [{name, version, replaced, backup}], ensured, notes}`. Admin. |
| asset_remove | DELETE /servers/{s}/assets/{name} | moves the folder to `.kora/asset-backups`, drops its `ensure` line and record → `{notes}`. Admin. |
| asset_downloads | GET /assets/downloads | `{found, folder, zips: [{name, size, modified_unix}]}`: `*.zip` in the Downloads folder changed in the last 24 h. **Admin, local only.** |

### 2.2 Conventions

- JSON, snake_case. Errors: `{"error": {"code": "not_found", "message": "..."}}` with a matching
  HTTP status. Codes: `bad_request`, `unauthorized`, `forbidden`, `not_found`, `conflict`,
  `needs_elevation` (carries `command: ["kora", ...]`), `internal`.
- Secrets (licence key, database passwords, webhook URL) are accepted but never returned.
- Jobs: a slow operation answers `202 {"job": "<id>"}`. `GET /jobs/{id}` returns
  `{id, op, server, state: "running"|"done"|"failed", events: [...], result, error}`.
  `GET /jobs/{id}/events` is SSE with `event: progress|done|failed`; it replays past events first.
  One mutating job per server at a time (`conflict` otherwise).
- `JobRegistry` lives in `kora_core::api::jobs` and uses std threads and channels (no tokio).

### 2.3 Local security (phase 3)

- Bind `127.0.0.1` by default (`--bind`, `--port`, default port 47120).
- `kora panel` creates a random token, opens `http://127.0.0.1:<port>/#token=<token>`; the page
  posts it to `/session` and gets an `HttpOnly; SameSite=Strict` cookie. `Authorization: Bearer`
  is also accepted (hub, scripts).
- Every request: the `Host` header must be the bound address (DNS rebinding). Non-GET requests:
  `Origin` must match and an `X-Kora` header must be present (CSRF).
- Response headers: strict CSP (`default-src 'self'`, no inline script, no external origins),
  `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `frame-ancestors 'none'`.

### 2.4 Remote access (phase 6)

Off by default: nothing above changes for `kora panel` as it is today.

- **Binding.** `kora panel --bind <ip>` (default `127.0.0.1`). A non-loopback bind needs remote access on
  (`--remote` for one run, or `enabled` in `<kora home>/panel.json`), otherwise the panel refuses to
  start. It then prints that traffic is plain HTTP and should only cross a private network (Tailscale,
  WireGuard) or an SSH tunnel. `0.0.0.0` / `::` also needs `allowed_hosts` or a known MagicDNS name and
  prints a louder warning. Allowed `Host` values: the bound address, `localhost` (loopback binds), the
  machine's MagicDNS names (from tailscaled), and `allowed_hosts` from the settings (each on the
  panel's port); the extra names only count while remote access is on. `Origin` follows the same rule.
- **Who is a remote request?** Any peer that is not a loopback address. A request with no peer
  information (tests calling the router) counts as local. Loopback requests keep today's rules: the
  launch token or its session, and they are admin.
- **Identity providers.**
  - *Token*: `kora panel user add <name> --role <role>` prints a random 256-bit token once; only
    `sha256(salt || token)` is stored; compared in constant time against every user. Sent as
    `Authorization: Bearer` or exchanged at `POST /session {token}` for the usual cookie. The launch
    token is not accepted from a remote peer, and a remote token is not accepted on loopback.
  - *Tailscale*: `POST /session {}` from a remote peer makes the panel ask the local tailscaled
    `whois` for the peer address (Linux: HTTP over `/var/run/tailscale/tailscaled.sock`, otherwise and on
    Windows `tailscale whois --json`). The login name must be in `tailscale_users`. Identity headers
    (`Tailscale-User-Login`, `X-Forwarded-*`) are never read. Tagged devices have no login and are refused.
- **Roles** (`remote/roles.rs`, an exhaustive `match` over `Op`, so a new op must be classified):
  - *viewer*: status and lists (GET), including job results. No logs, no console.
  - *operator*: viewer + start/stop/restart, update preview/apply/auto, ensure, scan, resource plan,
    log list/tail, console output (SSE), webhook test, port/disk checks, install log.
  - *admin*: everything else, subject to the next two rules.
  - *local-only* (refused remotely for every role): `remote_get/set`, `users_list/add/remove`,
    `server_open_folder`.
  - *remote-destructive* (refused remotely, even for admins, unless `allow_remote_destructive` is on):
    `server_delete`, `server_unregister`, `uninstall_run`, `app_uninstall`, `console_send`,
    `install_start`, `self_update_apply`, `schedule_set`, `schedule_remove`. Turning it on does not
    skip the typed-name confirmation the handlers enforce.
- **Sessions.** Same cookie flow for local and remote. A session records provider, name and (remote) the
  role it was made with; every request looks the user up again, so a removed user, a changed role, or
  Tailscale being switched off ends the session (401, the UI shows its sign-in screen). A cookie made for a
  remote peer does not work from loopback, and the other way round. Idle timeout 12 h, absolute lifetime
  7 days, for all sessions. Remote sign-in failures have their own limiter (10 per 60 s), so strangers
  cannot lock the owner out of the local one.
- **Who am I?** `GET /meta` carries `identity` (`null` when not signed in, else `{name, provider:
  local|tailscale|token, role}`), `remote` (this request came from another computer) and
  `allow_remote_destructive`. The UI's `useRole()` reads them to hide or disable what the role cannot do.
- **Settings file.** `<kora home>/panel.json`, mode 0600 on Unix, written atomically. The panel notices
  changes made by `kora panel user ...` while it runs.
- **Threat model.** See the remote access section of the wave-3 report: a viewer sees status only;
  an operator can run and update servers and read logs/console output; an admin can additionally change
  settings that touch the machine, but cannot delete, uninstall, type console commands, install or
  change schedules unless the owner switches that on locally, and can never manage remote access.
  The channel is plain HTTP: anyone who can read the network path can read tokens and cookies, so the
  path must be private. A malicious *local* user on a shared machine can read `panel.json` only if the
  file mode lets them (0600 prevents that on Unix).

## 3. Phases

1. **Baseline.** Golden test of the whole `kora --help` tree; `--hostname`, `--game-build`;
   setup codes removed (`--code` fails with a message); Mac code removed; docs (no SignPath, no
   setup codes); the start/stop drift between CLI and manager fixed.
2. **Shared API in core.** `kora_core::api` (Backend, Op, DTOs with serde, jobs); TUI and CLI on it.
3. **`kora-web`, local only.** axum server, token auth, embedded UI, all section 2.1 routes;
   `kora panel`.
4. **Wizard in the panel + root helper.**
5. **Monitoring.** Sampler (status, players from `/players.json`, CPU/RAM of the process tree,
   disk, ports, badge), 24 h history, alerts, opt-in panel service.
6. **Remote access.** Bind + identity providers (tailscaled whois, token), roles. See 2.4.
7. **Hub.** `RemoteBackend`, fleet view, capability-based version skew (`/meta`; v1 is additive only).

## 4. Parity tests

- `Op` → handler: exhaustive `match` in the router (compile time).
- CLI → Op: a test walks clap's `Cli::command()`; every subcommand and flag must be in a table as
  `Route(op, field)`, `CliOnly(reason)` or `Removed(reason)`.
- TUI → Op: `manager::Action`, wizard `Screen` and result `Action` each get a `parity()` function
  with no wildcard arm.
- Behaviour: the `FakeBackend` tests run directly and through HTTP.
- `kora --help` golden file: an existing flag cannot disappear silently.

## 5. Brand (web/)

| Token | Hex | Use |
|---|---|---|
| Kora Lime | #A8FF00 | primary action, active state (sparingly: one lime element per view) |
| Charcoal | #1D1E24 | cards, surfaces |
| Ink | #111315 | page background |
| Mist | #BCBDC0 | secondary text |
| White | #FFFFFF | primary text |
| Signal Blue | #4AA8FF | info |
| Success | #2ECC71 | online, done |
| Amber | #F5A623 | restarting, pending |
| Orange | #E67E22 | notices |
| Danger | #E74C3C | offline, errors, destructive |
| Steel | #95A5A6 | neutral |

Fonts, bundled (no CDN): Inter Tight (headings), Inter (body), JetBrains Mono (console, code).
Dark theme only.

## 6. Wave 1 work split (parallel, one branch each, merged into `feat/panel`)

| Branch | Owns | Must not touch |
|---|---|---|
| `panel/w1-baseline` | `crates/kora-cli`, `crates/kora-tui` except `manager/`, docs, CHANGELOG | `crates/kora-tui/src/manager/`, `crates/kora-core/src/api/`, deleting `kora-core/src/site` (done at merge) |
| `panel/w1-api` | `crates/kora-core/src/api/`, `crates/kora-tui/src/manager/`, serde derives in core | `crates/kora-cli`, the wizard |
| `panel/w1-web` | `crates/kora-web` (new), workspace `Cargo.toml` member line, `deny.toml` | everything else |
| `panel/w1-ui` | `web/` (new) | everything else |

## 7. The privileged helper (phase 4)

Optional. Without it nothing changes: privileged panel actions answer `needs_elevation` with the `kora ...` command to run as administrator. With it installed (once, `sudo kora helper install`), the panel runs those same commands itself. Full design: the module comment of `crates/kora-core/src/helper/mod.rs`.

- **Process.** `kora helper serve`, run as root by `kora-helper.service` (systemd). `kora helper install` copies the running `kora` to the root-owned `/usr/local/libexec/kora/kora` (so a user cannot swap the binary the helper runs; `serve` refuses to start from a binary or folder others can write), records the installing user (from `SUDO_UID`) in `/etc/kora/helper.json`, writes the unit and starts it. `kora helper uninstall` reverses it; `kora helper status` reports it. After updating Kora, run `sudo kora helper install` again to refresh the helper's copy.
- **Socket.** `/run/kora/helper.sock`, mode `0660`, owner root, group = the installing user's primary group (a dedicated `kora` group would need `groupadd` and a new login; the permission bits are only the first fence). Every connection is checked with `SO_PEERCRED`: uid must be root or the recorded user.
- **Protocol.** One JSON line request `{verb, args, env}`; answer = JSON lines `{"type":"progress","stream","line"}` then one `{"type":"result","ok","code"}`, or `{"type":"error","message"}` when refused. `ping` answers the version (this is `helper_installed` in `GET /system`).
- **Allow-list.** Verbs: `install`, `self-update`, `schedule` (`set`/`off`), `app` (`install`/`uninstall`), `uninstall`: exactly the commands the panel shows today. The arguments are re-parsed with the real clap `Cli` (injected into the core server as a validator), so the helper cannot run anything the CLI could not. `delete`, `service-host`, `--code`, `--license-key` on argv, unknown verbs and unknown environment names are refused. The child is the same `kora` (`--no-elevate <verb> <args>`), spawned with an argument vector (no shell) and a rebuilt environment (`PATH`, `HOME`, `USER`, `SUDO_USER`, `SUDO_UID`, `KORA_HOME` of the installing user).
- **Secrets.** Licence key and database admin password travel as `CFX_LICENSE_KEY` / `KORA_DB_ADMIN_PASSWORD` in the request's `env`, never on argv, never logged, masked if echoed.
- **Panel side.** `kora_web::elevated::privileged(command, env, on_line)`: helper reachable -> run there (lines become job events); otherwise the usual `needs_elevation`. Job-type actions (install, uninstall, self-update) become jobs; app install/removal and schedule changes run inside the request.
- **Windows.** Not implemented yet. Planned: named pipe `\\.\pipe\kora-helper` with a DACL for the installing user's SID, served by a Windows service started through `kora service-host`-style machinery, same protocol and allow-list. Until then `kora helper install` says it is unavailable and the panel keeps answering `needs_elevation`.
