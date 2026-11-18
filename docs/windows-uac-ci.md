# Testing the Windows permission (UAC) prompt and the service's refusals in CI

Workflow: `.github/workflows/windows-uac.yml`. Scripts: `scripts/smoke/15-installer-uac.ps1` (the scenarios),
`scripts/smoke/lib-accounts.ps1` (test accounts, filtered tokens, `Invoke-AsAccount`, panels as other accounts),
`scripts/smoke/svc-client.ps1` (one request to the service's pipe), `scripts/smoke/uac-answer.ps1` (answers the real
prompt through UI Automation), `scripts/smoke/vm-setup-uac-runner.ps1` (one-time VM setup). The MSI itself is tested by
`scripts/smoke/14-msi.ps1` (`windows-smoke.yml`, job `msi`); this workflow is about prompts and identities.

There is **one** permission prompt left in Kora: the installer's. `install.ps1` runs the checked MSI with
`Start-Process -Verb RunAs` (one prompt), and `kora service install` does the same through its own checked download.
The panel and the service never ask. The earlier version of this workflow tested the UAC worker (`kora admin-worker`,
S1 to S5 about its prompt, pipe and lifetime); that worker no longer exists and its scenarios are gone.

**Status: written without being able to run any Windows code.** Every assumption about Windows behaviour is listed at the
end; every failure prints evidence (policy values, token elevation type and integrity, the service state, the System
event log).

## What a hosted runner can and cannot do

| Need | `windows-latest` | Self-hosted VM |
|---|---|---|
| An **unelevated process of a normal administrator** (split token) | The job's account is an elevated administrator, with UAC normally off (`EnableLUA=0`; the script prints it). A *filtered copy* of the job's token (`runas /trustlevel:0x20000`, see `Open-FilteredProcess`: Administrators deny-only, privileges dropped) stands in for it. | Yes, for real: the runner account is a normal administrator signed in interactively with UAC on. |
| The **consent prompt** | No interactive desktop; with `EnableLUA=0` nothing prompts (`Start-Process -Verb RunAs` just runs). | Yes: on the normal desktop (`PromptOnSecureDesktop=0`); `uac-answer.ps1` clicks it. |
| A **standard account** and a **second administrator** | Yes: `net user` + secondary logon (`Invoke-AsAccount`, `Open-KoraPanel -Kind Account`). | Yes. |

## Scenarios

| Scenario | Hosted | VM | Checklist |
|---|---|---|---|
| P1 (hosted) `install.ps1 -System` runs the checked MSI from a local release folder: service running, program in Program Files | runs | | S1 |
| P1 (VM) the installer's prompt appears **once**, answer **No**: nothing installed | | runs | S1 |
| P2 (VM) the installer's prompt, answer **Yes**: the service is installed and no further prompt shows | | runs | S1 |
| R1 an account that is not on the list: `service status` says "serves this account: no"; its request is refused (or the pipe is closed to it) | runs | runs | S3 |
| R2 an **unelevated token of an administrator** who is not on the list is refused; after `kora service allow <sid>` it is served (medium integrity is enough, nothing elevated); after `deny` refused again | runs | runs | S3 |
| R3 the panel of an allowed standard account reports `admin_service: ready`, a not-allowed one `not_allowed`, and no prompt appears either way | runs | runs | S3, S8 |

Not automated here: `kora service install`'s own download (needs a published release; `install.ps1` exercises the same
`msiexec` step against a local release folder), "nobody answers the installer's prompt" (the script that waits for
msiexec has no timeout of its own: Windows keeps the prompt for a long time), the whole install through the service
(`14-msi.ps1` drives its steps), and everything in `docs/windows-test-checklist.md` that needs a real desktop.

The VM job is label-gated (`runs-on: [self-hosted, windows, kora-uac-vm]`) **and** variable-gated (`vars.KORA_UAC_VM ==
'true'`) so it is skipped, not queued for ever, where no such runner exists, and it never runs for pull requests from
forks (it runs repository code as an administrator on that machine).

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
   account's Startup folder. Git, Python and the .NET SDK must be installed on the VM (checkout, the local release
   server, WiX).
5. Check the runner is unelevated: in that window `whoami /groups` shows `Mandatory Label\Medium Mandatory Level` and
   Administrators as "Group used for deny only".
6. Set the repository variable `KORA_UAC_VM` to `true`.
7. Take a snapshot. Run the workflow with *Run workflow* once and read the summary table.
8. If the prompt is found but a button is not clicked: on the VM run `uac-answer.ps1 -Answer Dump -ResultFile
   C:\kora-uac\dump.json` while a prompt is showing; the file holds the prompt's UI tree. Adjust the names in `uac-answer.ps1`.
9. Security notes: the driver task runs a script the job copies in, at high integrity, as the runner account. Anyone who
   can run a job there can do that anyway; this is why the VM is dedicated and the job never runs for fork pull
   requests. The runner account's password sits in the registry for the auto sign-in.

## Assumptions to check on the first runs

When a scenario fails, look here first.

1. The hosted job's token is elevated (type 2). `EnableLUA` there is unknown to me; the script prints it.
2. `runas /trustlevel:0x20000` starts `powershell.exe` with Administrators deny-only and privileges dropped (measured on
   Windows 11: integrity stays High). A token made by hand with `CreateRestrictedToken` did not work: the handle could not
   be adjusted, and the child it started died with 0xC0000142 or hung at start.
3. Secondary logon (`Start-Process -Credential -LoadUserProfile`) works from the runner for a standard account, and the
   account can write `C:\kora-uac` (`Initialize-UacWorkDir` grants Users modify).
4. The service's pipe is reachable from a filtered token of an allowed administrator (the descriptor lists the SID; the
   deny-only Administrators group must not matter) and closed or refusing for others. A client that is not allowed may
   get "access denied" at open (exit 5) instead of the service's refusal (exit 3): both are accepted.
5. `kora service allow|deny <SID>` takes effect within a second, also for the pipe's own access list: Windows keeps ONE
   descriptor per pipe name, so the service sets it on the pipe itself (`SetKernelObjectSecurity`) when the list changes.
6. `install.ps1 -Base http://127.0.0.1:47411 -System` accepts the local http base (https is required elsewhere), runs
   `msiexec` through `Start-Process -Verb RunAs` without a prompt on an elevated runner, and `-NoLaunch -NoShortcuts`
   are accepted by the script.
7. The prompt on the normal desktop is a top-level window of `consent.exe` found through its process id; its buttons are
   named `Yes` and `No` (English); a high-integrity driver can drive it (UIPI); a locked session has no prompt.
8. `schtasks /run /tn KoraUacDriver` is allowed for the non-elevated owner of an interactive, highest-privileges task.
9. `ConsentPromptBehaviorAdmin` and `PromptOnSecureDesktop` take effect without a restart (only `EnableLUA` needs one).
10. Account names `koraadm`/`korastd`, the folder `C:\kora-uac` and the local port 47411/47421 are free on the machine.
