# Testing the Windows permission (UAC) prompt in CI

Workflow: `.github/workflows/windows-uac.yml`. Scripts: `scripts/smoke/14-uac.ps1` (the scenarios),
`scripts/smoke/lib-accounts.ps1` (reusable helpers), `scripts/smoke/uac-answer.ps1` (answers the real prompt through
UI Automation), `scripts/smoke/vm-setup-uac-runner.ps1` (one-time VM setup).

**Status: written without being able to run any Windows code.** The scripts parse (PowerShell 7.7) and pass
PSScriptAnalyzer with the settings `windows-smoke.yml` uses; the C# helper compiles. Everything about how Windows
behaves is an assumption until the first runs: they are numbered in [Assumptions to check](#assumptions-to-check-on-the-first-runs)
and every failure prints the evidence to settle them (policy values, token elevation type, integrity level and group
SIDs of the job, the panel and the worker, the process tree, the `kora-admin-*` pipes with their SDDL, the System and
Application event logs).

## What a hosted runner can and cannot do

| Need | `windows-latest` | Self-hosted VM |
|---|---|---|
| An **unelevated process of a normal administrator** (split token) | The job's account is an elevated administrator. With UAC off (`EnableLUA=0`, which I could not confirm; the script prints it) there is **no** split token. Turning it on needs a **restart** (Microsoft: "takes effect only after a restart"), and a job cannot survive one. A *filtered copy* of the job's own token can be made by hand (`CreateRestrictedToken` with `LUA_TOKEN`: Administrators deny-only, medium integrity, owner = the user); that is what `KoraTok.LaunchFiltered` does. | Yes, for real: the runner account is a normal administrator signed in interactively with UAC on. |
| The **consent prompt** itself | No: there is no interactive desktop for the prompt, and with `EnableLUA=0` nothing prompts. | Yes: the prompt is on the normal desktop (`PromptOnSecureDesktop=0`) and `uac-answer.ps1` clicks it. |
| **Auto-approve** (`ConsentPromptBehaviorAdmin=0`, "elevate without prompting") | Only meaningful when `EnableLUA=1`: the policy value itself needs no restart (assumption 9). | Yes. |
| **Auto-deny** | `ConsentPromptBehaviorUser=0` denies elevation for *standard* users: usable when `EnableLUA=1`. Administrators have no "deny" setting; "No" is a click. | Yes, plus a real "No". |
| A **standard account** and a **second administrator** | Yes, `net user` + secondary logon (`Start-Process -Credential`). | Yes. |
| **Another account's credentials** typed into the prompt | No (needs the credential UI). | Yes (best effort; see assumption 13). |

So the hosted job answers a smaller question that is still worth asking on every pull request, and the VM job answers
the real one:

* **hosted**: *an unelevated panel (a hand-made filtered administrator token) and a standard account must never end up
  with an administrator worker, never hang, and never leave a worker or a stray process behind.* If the runner turns
  out to have UAC on (`EnableLUA=1`), the approved-by-policy scenarios run there as well; otherwise they are skipped
  with a `::notice::` that says why.
* **vm**: approved by policy, the real prompt with Yes / No / nobody answering / a standard account using an
  administrator's credentials, the pipe closed to the same user unelevated, and the worker following its panel.

The VM job is label-gated (`runs-on: [self-hosted, windows, kora-uac-vm]`) **and** variable-gated (`vars.KORA_UAC_VM ==
'true'`) so it is skipped, not queued for ever, where no such runner exists, and it never runs for pull requests from
forks.

## How the scenarios reach the real code

`14-uac.ps1` starts an unelevated `kora panel --foreground --no-open` and drives it over its HTTP API with the launch
token (a local caller may send it as `Authorization: Bearer`). It then does `POST /api/v1/self-update` from a copy of
`kora.exe` under `%ProgramFiles%\KoraUacTest\` (writable by administrators only): `self_update_needs_admin()` is then
true, so the panel (`handlers/maintenance.rs`) takes exactly the path the install wizard takes:
`elevated::privileged_as` → `winpipe::open_session` → ShellExecute `runas` → `kora admin-worker` → pipe + both peer
checks → `self-update --yes` run by the worker → job result. Polling `GET /api/v1/jobs/<id>` every 200 ms the script
also watches for the worker process (command line `admin-worker --pipe`), its token, `consent.exe` and the pipe, and
probes the pipe from an unelevated process.

Why self-update and not an install: it is the cheapest operation that needs administrator rights and has no side
effects on the machine (the worker contacts GitHub's release API and updates the throw-away copy at most). Its cost:
it depends on the GitHub API being reachable and not rate-limiting the runner (assumption 6).

### Scenarios (maps to `docs/windows-test-checklist.md`)

| Scenario | Hosted | VM | Checklist |
|---|---|---|---|
| S1 filtered administrator token cannot become the worker (job fails or hangs? it must **fail**, no worker left, panel alive) | runs | runs | A5 refusal side, W6 spirit |
| S2 split token, elevation approved by policy: job **done**, worker elevated, pipe refuses an unelevated process of the same user (W6), nothing left behind | only if `EnableLUA=1` | runs | A1-like, W6, A11-ish |
| S2b the panel is killed while the worker runs: the worker goes within 30 s | only if `EnableLUA=1` | runs | W10 |
| S3a real prompt, **Yes** | skipped | runs | A1, A2 |
| S3b real prompt, **No**: job fails with the guided "did not get permission"; ask again, Yes: finishes | skipped | runs | A2 |
| S3c real prompt **unanswered** for 5 min ("Nobody answered"); answering the stale prompt starts nothing (`-Slow`, always on in the VM job) | skipped | runs | A3 |
| S4 standard account with the policy denying elevation: no administrator update, no hang | runs | runs | A5 (refusal) |
| S5 standard account, an administrator's credentials typed into the prompt | skipped | runs (best effort) | A5, W5 (partly) |

Not automated (stay in the manual checklist): A1's "one prompt for the whole wizard" and the owned-by-you folder check
(needs a full install with a real FXServer download), A4 (killing the worker at the service step), A6 (non-English
paths), A7 (remote session), A8 (one prompt per action), A9 (Windows Server 2022: run the VM on that), A10, W1-W5,
W7-W9, W11, W12. W3 (the program cannot be swapped after the prompt) could be added to S2 with a rename attempt while
the worker runs; the worker is too short-lived for a reliable race, so it needs a longer step first.

## The VM variant, step by step

1. A Windows 11 or Windows Server 2022/2025 VM, no secrets on it, snapshot-able. It will run repository code as an
   administrator: keep it disposable and revert to the snapshot after a suspicious run.
2. In **Windows PowerShell 5.1 as administrator**, from a checkout: `.\scripts\smoke\vm-setup-uac-runner.ps1
   -RunnerPassword (Read-Host -AsSecureString)` (optionally `-RunnerUser`, default `koraci`). It creates the runner
   account (a normal administrator), enables automatic sign-in, switches UAC on with `PromptOnSecureDesktop=0`, stops
   the lock screen and sleep, and registers the scheduled task `KoraUacDriver` (interactive, highest privileges).
3. **Restart** (the UAC setting needs it). The VM signs in as the runner account.
4. Register the runner in the repository (Settings → Actions → Runners → New self-hosted runner → Windows) with the
   label `kora-uac-vm` and run it **interactively in a normal, not elevated, window** with `run.cmd` (not as a
   service: a service is session 0, which has no desktop and no prompt). Put a shortcut to `run.cmd` in the runner
   account's Startup folder so it comes back after a restart. Git must be installed on the VM (checkout).
5. Check the runner is unelevated: in that window `whoami /groups` shows `Mandatory Label\Medium Mandatory Level` and
   Administrators as "Group used for deny only".
6. Set the repository variable `KORA_UAC_VM` to `true` (Settings → Secrets and variables → Actions → Variables).
7. Take a snapshot. Run the workflow with *Run workflow* once and read the summary table.
8. If the prompt is found but a button is not clicked: on the VM run `uac-answer.ps1 -Answer Dump -ResultFile
   C:\kora-uac\dump.json` while a prompt is showing (start one with any "Run as administrator"); the file holds the
   prompt's UI tree. Adjust the names in `uac-answer.ps1`.
9. Security notes: the driver task runs a script the job copies in, at high integrity, as the runner account. Anyone who
   can run a job there can do that anyway (they are an administrator); this is why the VM is dedicated and the job never
   runs for fork pull requests. The runner account's password sits in the registry for the auto sign-in.

## Reusing the helpers for the installed service (the next version's normal path)

`lib-accounts.ps1` is independent of the worker: dot-source it from an **elevated** script.

* `Initialize-TestAccount -Name x [-Administrator]` / `Clear-TestAccount`: a standard or administrator account with a
  random masked password; `.Credential` is a `PSCredential`.
* `Open-KoraPanel -Kind Self|Filtered|Account ...` starts `kora panel` as the current token, a hand-made filtered token
  of this administrator, or another account. `Invoke-PanelApi` / `Wait-PanelJob` drive it. For the service: start the
  panel as `Filtered` (same user, unelevated: must be **served**), as `Account` with a standard user (not the installing
  account: must be **refused** by `helper::winhelper::check_client`), and as an administrator that is not the installing
  account; call an operation that goes through the service and assert the answers. Use
  `Write-PipeDiagnostic` (`\\.\pipe\kora-helper` has the SDDL of `pipe_sddl`) and `Test-PipeAsUnelevated` from
  `14-uac.ps1` (change the pipe name).
* `Get-TokenFact -ProcessId` / `Write-TokenDiagnostic` / `Write-SystemDiagnostic`: token elevation type, integrity,
  groups (deny-only flagged), privileges, processes, pipes, event-log tail.
* `Initialize-UacPolicy` / `Restore-UacPolicy`: change and restore `ConsentPromptBehaviorAdmin/User` and
  `PromptOnSecureDesktop`.

## Assumptions to check on the first runs

Each is something about Windows or the panel that I could not run. When a scenario fails, look here first.

1. The hosted job's token is elevated (type 2). `EnableLUA` there is **unknown to me**; the script prints it. If it is
   1, S2/S2b run on hosted too (good); if the first line of the log says UAC is on and S2 fails, assumption 8 or 9 is
   the suspect.
2. `CreateProcessAsUser` with a `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE | LUA_TOKEN)` copy of the job's own token
   works without `SeAssignPrimaryTokenPrivilege` (documented for restricted copies of the caller's token), starts
   `powershell.exe` from the job's service session, and the child reports `elevated: False`, integrity 8192,
   Administrators deny-only. The reported *elevation type* of such a hand-made token may be 1 or 3 (unknown).
3. An unelevated `kora panel --foreground --no-open --port N` starts under the filtered token, binds loopback and
   writes `panel-running.json` (with `token`) into `KORA_HOME`; the wrapper copies it.
4. A local request with `Authorization: Bearer <launch token>` and no `X-Kora` header is accepted for `POST` (read from
   `security.rs`); `POST /api/v1/self-update` answers `202 {"job": id}` and `GET /api/v1/jobs/<id>` has `state`
   `running|done|failed`.
5. `self_update_needs_admin()` is true for the Program Files copy under every unelevated token used here
   (`su::dir_writable` tries to write; a deny-only Administrators group must not allow it).
6. The elevated `self-update --yes` is allowed by the worker's validator (`kora-cli/src/main.rs`) and reaches GitHub's
   unauthenticated API (60 requests/hour per IP: a busy shared runner IP can be rate-limited, which fails S2 for a
   non-UAC reason). On a build whose version is older than the latest release it really replaces the *copy*.
7. With `EnableLUA=0`, `ShellExecute runas` from a non-elevated process starts the worker with the *same* (unelevated)
   token, and the handshake fails (pipe DACL asks for the enabled Administrators group): S1 expects a failed job. If
   Windows instead elevates or refuses differently, S1's expectation changes (the safety properties stay).
8. The secondary logon service (`Start-Process -Credential`) gives a local administrator its **filtered** token when
   UAC is on, and a profile loads (`-LoadUserProfile`).
9. `ConsentPromptBehaviorAdmin`, `ConsentPromptBehaviorUser` and `PromptOnSecureDesktop` take effect without a restart
   (only `EnableLUA` needs one).
10. With `ConsentPromptBehaviorUser=0` a standard user's `runas` fails promptly instead of waiting (S4 allows up to 180 s
    and only requires "not done").
11. The prompt on the normal desktop is a top-level window of `consent.exe` that UI Automation finds through the
    process id; its buttons are named `Yes` and `No` (English); a medium-integrity process cannot drive it (UIPI), a
    high-integrity one can; Alt+Y / Alt+N are the fallback. A locked or disconnected session has no prompt.
12. `schtasks /run /tn KoraUacDriver` is allowed for the non-elevated owner of an interactive, highest-privileges task.
13. The credential prompt of a standard user: the account is a selectable list item containing the user name and a
    password box (`IsPassword`) supports `ValuePattern.SetValue`. Likely needs tuning with the `Dump` output.
14. `OpenProcess(QUERY_LIMITED_INFORMATION)` + `OpenProcessToken(TOKEN_QUERY)` on another user's panel works from an
    elevated job (hosted) but may be refused for the elevated *worker* from an unelevated job (VM): then the token is
    reported "unreadable" with a warning and S2 relies on the job finishing.
15. `Get-Acl \\.\pipe\name` may not be readable; the SDDL print is diagnostic only. WMI's `Win32_Process.CommandLine` is
    visible for elevated processes (WMI runs as SYSTEM).
16. The worker is alive long enough to be seen (S2b needs it for ~100 ms polling; a very fast `self-update` that is
    already up to date may be too quick: S2b then only warns "W10 not exercised" and needs a slower operation).
17. Account names `koraadm`/`korastd` and the folder `C:\kora-uac` are free on the machine.
