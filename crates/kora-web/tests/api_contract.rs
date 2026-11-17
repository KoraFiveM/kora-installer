//! The contract between the Rust description of the API (`kora_core::api::spec`, the source of the
//! generated TypeScript) and what the real routes answer.
//!
//! For every operation that has a case below, the real handler runs (fake backend, offline seams, a
//! throwaway Kora home) and
//! * the request body of the case must read as the declared request type,
//! * the answer must read as the declared response type AND write back to the very same JSON (so a
//!   field the type does not know, or a `null` where the type leaves the field out, fails here),
//! * a job's events and result must read as the declared event and result types, a failed job's error
//!   as `JobError`.
//!
//! Every error answer of every operation must read as `ErrorEnvelope`.
//!
//! An operation without a case is listed in `UNVERIFIED` with the reason; the list is printed by the
//! test, and a typed operation must be in one place or the other.
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, Response};
use kora_core::api::fake::FakeBackend;
use kora_core::api::install::github::{GithubEnv, GithubUser, LoginFlow, OwnerChoice};
use kora_core::api::install::InstallEnv;
use kora_core::api::spec::{spec, Body as SpecBody, Resp, Ty};
use kora_core::api::wire::{ErrorEnvelope, JobError, JobRecord};
use kora_core::api::{Method, Op};
use kora_core::artifacts::{ArtifactBuild, ArtifactList};
use kora_core::firewall::ports::PortProber;
use kora_core::installer::{DbAdmin, InstallOutcome, InstallRequest, ProgressEvent, ProgressSink, StepStatus};
use kora_core::space::DiskInfo;
use kora_web::handlers::{assets, github, hub, install, resources, servers, system, toolchain, updates};
use kora_web::remote::Remote;
use kora_web::{router, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADDR: &str = "127.0.0.1:47120";
const TOKEN: &str = "testtoken";

// ---------------------------------------------------------------------------------------------
// The machine, replaced: nothing here touches the network, `gh`, services or the real Kora home.
// ---------------------------------------------------------------------------------------------

struct Env;
impl PortProber for Env {
    fn tcp_free(&self, _: u16) -> bool {
        true
    }
    fn udp_free(&self, _: u16) -> bool {
        true
    }
}
impl DiskInfo for Env {
    fn free_bytes(&self, _: &Path) -> std::io::Result<u64> {
        Ok(100 << 30)
    }
    fn volume_key(&self, _: &Path) -> String {
        "v".into()
    }
}
impl InstallEnv for Env {
    fn prober(&self) -> &dyn PortProber {
        self
    }
    fn disk(&self) -> &dyn DiskInfo {
        self
    }
    fn db_login(&self, _: Option<&DbAdmin>) -> Result<(), String> {
        Ok(())
    }
    fn artifacts(&self) -> Result<ArtifactList, String> {
        let b = |n: u32| ArtifactBuild { build: n, url: format!("https://example.invalid/{n}/fx.tar.xz"), broken: None };
        Ok(ArtifactList { recommended: Some(b(7000)), latest: Some(b(7100)), all: vec![b(7100), b(7000)], broken: Default::default() })
    }
    fn is_elevated(&self) -> bool {
        false
    }
    fn admin_reasons(&self, _: &InstallRequest) -> Vec<String> {
        vec![]
    }
    fn saved_load(&self) -> Option<InstallRequest> {
        None
    }
    fn saved_save(&self, _: &InstallRequest) -> bool {
        true
    }
    fn saved_clear(&self) {}
    fn run(&self, req: &InstallRequest, sink: &mut dyn ProgressSink) -> Result<InstallOutcome, String> {
        sink.event(ProgressEvent { step: "prepare".into(), title: "Getting ready".into(), status: StepStatus::Running, detail: "…".into(), percent: 10 });
        sink.event(ProgressEvent { step: "prepare".into(), title: "Getting ready".into(), status: StepStatus::Done, detail: String::new(), percent: 100 });
        Ok(InstallOutcome {
            dir: req.dir.clone(),
            connect: "localhost:30120".into(),
            txadmin_url: "http://127.0.0.1:40120".into(),
            notes: vec!["ok".into()],
            name: req.name.clone(),
            start_attempted: false,
            running: false,
            txadmin_up: false,
            game_listening: false,
            launch_problem: None,
            log_tail: vec![],
            admin_user: Some("kora".into()),
            admin_password_file: Some(req.dir.join("admin.txt")),
        })
    }
}
impl GithubEnv for Env {
    fn auth(&self) -> GithubUser {
        GithubUser::ready("octocat")
    }
    fn owners(&self) -> Result<Vec<OwnerChoice>, String> {
        Ok(vec![
            OwnerChoice { login: "octocat".into(), is_org: false, can_fork: true, reason: None },
            OwnerChoice { login: "acme".into(), is_org: true, can_fork: false, reason: Some("No permission".into()) },
        ])
    }
    fn begin_login(&self) -> Result<LoginFlow, String> {
        Ok(LoginFlow { code: "ABCD-1234".into(), url: "https://github.com/login/device".into(), wait: Box::new(|| Ok(Some("octocat".into()))) })
    }
    fn resolve_owner(&self, _: Option<&str>) -> Result<String, String> {
        Ok("octocat".into())
    }
}
impl kora_core::api::toolchain::ToolchainEnv for Env {
    fn tools(&self) -> Vec<kora_core::tools::ToolStatus> {
        use kora_core::problem::Tool;
        use kora_core::tools::{Source, ToolStatus};
        vec![
            ToolStatus {
                tool: Tool::Git,
                name: "git",
                label: "Git",
                installed: true,
                version: Some("2.45.0".into()),
                source: Some(Source::System),
                path: Some("/usr/bin/git".into()),
                broken: false,
                can_install: false,
            },
            ToolStatus { tool: Tool::Gh, name: "gh", label: "the GitHub CLI", installed: false, version: None, source: None, path: None, broken: false, can_install: true },
        ]
    }
    fn install(&self, tool: kora_core::problem::Tool, say: &mut dyn FnMut(&str, f32)) -> Result<kora_core::tools::ToolStatus, String> {
        say("Downloading", 0.5);
        let mut s = self.tools().remove(0);
        s.tool = tool;
        Ok(s)
    }
    fn database(&self) -> kora_core::api::toolchain::DbState {
        kora_core::api::toolchain::DbState { installed: true, running: false, name: Some("mariadb".into()), version: Some("11".into()), unknown: false }
    }
    fn start_database(&self, say: &mut dyn FnMut(&str)) -> Result<String, String> {
        say("Starting");
        Ok("The database is running.".into())
    }
    fn admin(&self) -> kora_core::api::toolchain::AdminState {
        kora_core::api::toolchain::AdminState { elevated: true, helper: false, service: false }
    }
}

struct NoNet;
impl kora_core::hub::Transport for NoNet {
    fn send(&self, _: &kora_core::hub::HttpRequest) -> Result<kora_core::hub::HttpResponse, String> {
        Err("offline".into())
    }
}

/// One-time process setup: a throwaway Kora home, offline seams, fake maintenance.
fn seams() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let home = std::env::temp_dir().join(format!("kora-contract-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("KORA_HOME", &home);
        kora_core::api::maintenance::set_override(Some(kora_core::api::maintenance::fake::FakeMaintenance::new(false)));
        toolchain::set_env(Some(Arc::new(Env)));
        install::set_env(Some(Arc::new(Env)));
        github::set_env(Some(Arc::new(Env)));
        resources::set_catalog_loader(Some(Arc::new(|| Ok((kora_core::catalog::Catalog::embedded().unwrap(), None)))));
        updates::set_auto_runner(Some(Arc::new(|_| Ok(Default::default()))));
        servers::set_opener(Some(Box::new(|_| Ok(()))));
        system::seam::set_headless(Some(true));
        hub::set_store_path(Some(home.join("nodes.json")));
        hub::set_transport(Some(Arc::new(NoNet)));
        assets::set_downloads_dir(Some(home.join("downloads")));
    });
}

// ---------------------------------------------------------------------------------------------
// Fixtures for the fake backend
// ---------------------------------------------------------------------------------------------

fn preview(f: &FakeBackend) {
    use kora_core::update::manual::{ChangeStatus, ResourceChange, UpdatePreview};
    f.state().preview = Some(Ok(UpdatePreview {
        resources: vec![ResourceChange {
            id: "kora_hud".into(),
            from: "1111111".into(),
            to: "2222222".into(),
            behind: 2,
            changelog: vec!["Fix the hud".into()],
            needs_restart: true,
            status: ChangeStatus::Clean,
            note: None,
        }],
        new_release: Some("1.4.0".into()),
        notes: vec!["The built-in release list was used.".into()],
    }));
}

fn apply(f: &FakeBackend) {
    use kora_core::update::manual::UpdateEvent;
    use kora_core::update::{DeferReason, SkipReason, UpdateReport, Updated};
    let mut s = f.state();
    s.apply_events = vec![
        UpdateEvent::Planned(vec!["kora_hud".into()]),
        UpdateEvent::Resource { id: "kora_hud".into(), state: "updating".into() },
        UpdateEvent::Snapshot("snap1".into()),
        UpdateEvent::Health("ok".into()),
        UpdateEvent::Done,
    ];
    s.apply = Some(Ok(UpdateReport {
        server: "prod".into(),
        updated: vec![Updated { id: "kora_hud".into(), from: "1111111".into(), to: "2222222".into() }],
        skipped: vec![("kora_lib".into(), SkipReason::Dirty)],
        deferred: vec![("kora_core".into(), DeferReason::PlayersOnline(2))],
        rolled_back: vec![],
        sql_flagged: vec![],
        needs_attention: vec![],
        forks_synced: vec![],
        forks_left_alone: vec![],
        not_attempted: vec![],
        held: vec![],
        notes: vec!["note".into()],
    }));
}

fn snapshots(f: &FakeBackend) {
    f.state().snapshots = vec![kora_core::update::manual::SnapshotInfo { id: "snap1".into(), created_unix: 1_700_000_000, resources: vec!["kora_hud".into()] }];
}

fn catalog(f: &FakeBackend) {
    f.state().catalog = Some((kora_core::catalog::Catalog::embedded().unwrap(), vec!["kora_lib".into()]));
}

fn add_plan(f: &FakeBackend) {
    use kora_core::addresources::{AddOutcome, AddPlan, PlanItem};
    let mut s = f.state();
    s.add_plan = Some(Ok(AddPlan {
        items: vec![PlanItem {
            id: "kora_hud".into(),
            rel: "[kora]/kora_hud".into(),
            repo: "KoraFiveM/kora_hud".into(),
            branch: "main".into(),
            commit: Some("abc".into()),
            fork: false,
            third_party: false,
            subpath: None,
            dependency: false,
            repository: "https://github.com/KoraFiveM/kora_hud".into(),
        }],
        already: vec![],
        ensure: vec!["kora_hud".into()],
    }));
    s.add_result = Some(Ok(AddOutcome { installed: vec!["kora_hud".into()], failed: vec![("x".into(), "why".into())], ensured: vec!["kora_hud".into()], notes: vec![] }));
}

fn pr(f: &FakeBackend) {
    let mut s = f.state();
    s.signed_in = true;
    s.pr_plan = Some(Ok(pr_plan()));
}

fn pr_plan() -> kora_core::feedback::PrPlan {
    kora_core::feedback::PrPlan {
        resource: "kora_hud".into(),
        upstream: "KoraFiveM/kora_hud".into(),
        base: "main".into(),
        head_owner: "octocat".into(),
        head_branch: "fix".into(),
        ahead: 1,
        dir: "/srv/dev/resources/kora_hud".into(),
    }
}

fn signed_in(f: &FakeBackend) {
    f.state().signed_in = true;
}

fn notify(f: &FakeBackend) {
    f.state().notify.insert("prod".into(), kora_core::notify::NotifyConfig { discord_webhook: Some("https://discord.com/api/webhooks/123456/SECRETTOKEN".into()) });
}

fn schedule(f: &FakeBackend) {
    use kora_core::service::{Preset, RunAs, ScheduleConfig};
    f.state().schedule.insert("dev".into(), ScheduleConfig { preset: Preset::Weekly, time: "04:00".into(), weekday: Some(2), run_as: RunAs::InstallingUser, enabled: true });
}

fn assets_fixture(f: &FakeBackend) {
    use kora_core::paidassets::hints::{EntitlementHint, HintKind};
    use kora_core::paidassets::{AssetsView, PaidAsset};
    f.state().assets.insert(
        "prod".into(),
        AssetsView {
            assets: vec![PaidAsset { name: "shop".into(), path: "[paid]/shop".into(), version: Some("1.0.0".into()), imported_unix: Some(1_700_000_000), present: true, has_fxap: true, backups: 1 }],
            hints: vec![EntitlementHint {
                kind: HintKind::Entitlement,
                resource: Some("shop".into()),
                line: "You lack the required entitlement".into(),
                fix: "Sign in with the right Cfx.re account".into(),
            }],
        },
    );
}

fn branch_state(id: &str) -> kora_core::branches::ResourceBranch {
    kora_core::branches::ResourceBranch {
        id: id.into(),
        folder: format!("[kora]/{id}"),
        state: kora_core::branches::State::Branch,
        branch: Some("fix-hud".into()),
        pr: None,
        default_branch: "dev".into(),
        label: "branch fix-hud".into(),
        changed_count: 1,
        stashes: 0,
        note: None,
        problem: None,
    }
}

fn branches_overview(f: &FakeBackend) {
    f.state().branches = Some(Ok(kora_core::branches::Reply::Overview(vec![branch_state("kora_hud")])));
}

fn branches_choices(f: &FakeBackend) {
    use kora_core::git::branches::{Branches as BranchLists, Stash};
    f.state().branches = Some(Ok(kora_core::branches::Reply::Choices(Box::new(kora_core::branches::Choices {
        resource: branch_state("kora_hud"),
        branches: BranchLists { current: Some("fix-hud".into()), default_branch: "dev".into(), local: vec!["dev".into()], fork: vec!["fix-hud".into()], upstream: vec!["dev".into()] },
        stashes: vec![Stash { index: 0, message: "WIP".into(), branch: "fix-hud".into() }],
        notes: vec![],
    }))));
}

fn branches_prs(f: &FakeBackend) {
    use kora_core::git::branches::{PrSource, PullRequest};
    f.state().branches = Some(Ok(kora_core::branches::Reply::Prs(kora_core::branches::PrChoices {
        prs: vec![PullRequest {
            number: 12,
            title: "Fix".into(),
            author: "sam".into(),
            source: PrSource::Upstream,
            repo: "KoraFiveM/kora_hud".into(),
            branch: "fix".into(),
            from: Some("sam/kora_hud".into()),
            draft: false,
            url: "https://github.com/x/y/pull/12".into(),
            updated: "2026-01-01".into(),
        }],
        notes: vec![],
    })));
}

fn switched() -> kora_core::branches::SwitchResult {
    use kora_core::git::branches::{Kind, Outcome, Status, Where};
    kora_core::branches::SwitchResult {
        resource: "kora_hud".into(),
        outcome: Outcome {
            status: Status::Switched,
            now: Where { kind: Kind::Branch, branch: Some("fix-hud".into()), pr: None, changed_files: vec![], changed_count: 0 },
            stashed: None,
            saved_branch: None,
            title: None,
            message: "Switched.".into(),
        },
        needs_restart: true,
        restart_note: Some("Restart the server.".into()),
    }
}

fn branches_switched(f: &FakeBackend) {
    f.state().branches = Some(Ok(kora_core::branches::Reply::Switched(Box::new(switched()))));
}

fn bulk_preview(f: &FakeBackend) {
    f.state().branches = Some(Ok(kora_core::branches::Reply::BulkPreview(kora_core::branches::BulkPreview {
        branch: "fix-hud".into(),
        rows: vec![kora_core::branches::BulkRow { id: "kora_hud".into(), has: true, from: Some("origin".into()), already: false, now: "the normal version".into(), changed_count: 0, note: None }],
        have: 1,
        missing: 0,
        notes: vec![],
    })));
}

fn bulk_apply(f: &FakeBackend) {
    use kora_core::branches::{BulkItem, BulkResult, BulkStatus};
    f.state().branches = Some(Ok(kora_core::branches::Reply::BulkApply(BulkResult {
        items: vec![BulkItem { id: "kora_hud".into(), status: BulkStatus::Switched, message: "Switched.".into(), stashed: None, files: vec![], problem: None }],
        switched: 1,
        needs_choice: 0,
        failed: 0,
        skipped: 0,
        needs_restart: true,
        restart_note: None,
    })));
}

// ---------------------------------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------------------------------

type Setup = fn(&FakeBackend);

struct Case {
    op: Op,
    /// Fixtures for the fake backend (each case gets a fresh one).
    setup: Option<Setup>,
    /// `{s}`, `{id}`, ... in the route.
    params: &'static [(&'static str, &'static str)],
    query: &'static str,
    body: Option<Value>,
}

fn case(op: Op, params: &'static [(&'static str, &'static str)], body: Option<Value>) -> Case {
    Case { op, setup: None, params, query: "", body }
}

fn with(mut c: Case, setup: Setup) -> Case {
    c.setup = Some(setup);
    c
}

fn query(mut c: Case, q: &'static str) -> Case {
    c.query = q;
    c
}

fn install_request(dir: &str) -> Value {
    json!({
        "mode": "production", "name": "Contract Server", "dir": dir, "hostname": "Contract Server",
        "license_key": "cfxk_0123456789abcdef0123456789abcdef_ab12cd",
        "ports": {"game": 31120, "txadmin": 41120}, "txadmin_public": false,
        "artifact_url": null, "game_build": null, "resources": "all", "db": "skip", "service": false, "firewall": false,
        "github_owner": null, "dry_run": false, "db_admin": null, "start": false, "cfx_identifier": null
    })
}

fn cases() -> Vec<Case> {
    let s = &[("s", "prod")];
    let dev = &[("s", "dev")];
    let home = std::env::temp_dir().join(format!("kora-contract-{}", std::process::id()));
    let dir = home.join("new-server").display().to_string();
    vec![
        case(Op::Meta, &[], None),
        case(Op::SessionCreate, &[], Some(json!({"token": TOKEN}))),
        case(Op::SystemInfo, &[], None),
        case(Op::RemoteGet, &[], None),
        case(Op::RemoteSet, &[], Some(json!({"allow_remote_destructive": false}))),
        case(Op::UsersList, &[], None),
        case(Op::UserAdd, &[], Some(json!({"name": "sam", "role": "operator"}))),
        case(Op::ServersList, &[], None),
        case(Op::ServerGet, s, None),
        case(Op::ServerControl, s, Some(json!({"action": "restart"}))),
        case(Op::ServersScan, &[], None),
        case(Op::ResourcesList, s, None),
        case(Op::ResourceEnsure, s, Some(json!({"name": "kora_hud", "restart": false}))),
        with(case(Op::UpdatePreview, s, Some(json!({"scope": "all"}))), preview),
        with(case(Op::UpdateApply, s, Some(json!({"scope": {"resources": ["kora_hud"]}, "restart": "when_quiet", "skip": [], "targets": {"kora_hud": "2222222"}}))), apply),
        case(Op::UpdateAuto, dev, None),
        with(case(Op::SnapshotsList, s, None), snapshots),
        with(case(Op::SnapshotRollback, &[("s", "prod"), ("id", "snap1")], Some(json!({"force": true}))), snapshots),
        with(case(Op::CatalogForServer, s, None), catalog),
        with(case(Op::ResourcesPlan, s, Some(json!({"ids": ["kora_hud"]}))), add_plan),
        with(case(Op::ResourcesAdd, s, Some(json!({"ids": ["kora_hud"]}))), add_plan),
        with(case(Op::WebhookGet, s, None), notify),
        case(Op::WebhookSet, s, Some(json!({"url": "https://discord.com/api/webhooks/123456/SECRETTOKEN"}))),
        with(case(Op::ScheduleGet, dev, None), schedule),
        case(Op::ScheduleSet, dev, Some(json!({"preset": "daily", "time": "04:00", "weekday": null, "run_as": "installing_user", "enabled": true}))),
        case(Op::PortsSet, dev, Some(json!({"game": 30140, "txadmin": 40140}))),
        with(case(Op::IssueReport, s, Some(json!({"resource": "kora_hud", "title": "Broken", "text": "It is"}))), signed_in),
        with(case(Op::PrPlan, dev, Some(json!({"resource": "kora_hud"}))), pr),
        with(case(Op::PrOpen, dev, Some(json!({"plan": pr_plan(), "title": "Fix", "text": "Please"}))), pr),
        case(Op::GithubStatus, &[], None),
        case(Op::GithubLogin, &[], None),
        case(Op::GithubOwners, &[], None),
        case(Op::CatalogGet, &[], None),
        case(Op::ArtifactsList, &[], None),
        case(Op::InstallDefaults, &[], None),
        case(Op::CheckPorts, &[], Some(json!({"game": 30120, "txadmin": 40120}))),
        case(Op::CheckDisk, &[], Some(json!({"dir": dir.clone()}))),
        case(Op::CheckDbLogin, &[], Some(json!({"admin": null}))),
        case(Op::CheckCfx, &[], Some(json!({"username": "nobody-here"}))),
        case(Op::InstallPlan, &[], Some(install_request(&dir))),
        case(Op::InstallStart, &[], Some(install_request(&dir))),
        case(Op::SelfUpdateCheck, &[], None),
        case(Op::AppStatus, &[], None),
        case(Op::AppInstall, &[], Some(json!({"desktop_shortcut": false, "dry_run": true}))),
        case(Op::AppUninstall, &[], Some(json!({"dry_run": true}))),
        case(Op::UninstallPlan, &[], None),
        case(Op::MetricsGet, s, None),
        query(case(Op::MetricsGet, s, None), "range=6h"),
        case(Op::AlertsList, &[], None),
        case(Op::NodesList, &[], None),
        case(Op::Fleet, &[], None),
        case(Op::ToolsStatus, &[], None),
        case(Op::ToolInstall, &[("tool", "gh")], None),
        case(Op::SystemCheck, &[], None),
        case(Op::DbStart, &[], None),
        case(Op::KeysList, &[], None),
        case(Op::JobsList, &[], None),
        case(Op::AuditList, &[], None),
        case(Op::KeyAdd, &[], Some(json!({"name": "Main", "license_key": "cfxk_0123456789abcdef0123456789abcdef_ab12cd"}))),
        case(Op::ProfileGet, &[], None),
        case(Op::ProfileSet, &[], Some(json!({"github_owner": null}))),
        with(case(Op::AssetsList, s, None), assets_fixture),
        case(Op::AssetDownloads, &[], None),
        with(case(Op::BranchesList, dev, None), branches_overview),
        with(case(Op::BranchChoices, &[("s", "dev"), ("name", "kora_hud")], None), branches_choices),
        with(query(case(Op::BranchChoices, &[("s", "dev"), ("name", "kora_hud")], None), "refresh=1"), branches_choices),
        with(case(Op::PrList, &[("s", "dev"), ("name", "kora_hud")], None), branches_prs),
        with(case(Op::BranchSwitch, &[("s", "dev"), ("name", "kora_hud")], Some(json!({"action": "branch", "branch": "fix-hud", "remote": "origin", "changes": "stash"}))), branches_switched),
        with(case(Op::StashRestore, &[("s", "dev"), ("name", "kora_hud")], Some(json!({"index": 0}))), branches_switched),
        with(case(Op::BranchBulkPreview, dev, Some(json!({"branch": "fix-hud", "fetch": true}))), bulk_preview),
        with(case(Op::BranchBulkApply, dev, Some(json!({"action": "default", "changes": "ask"}))), bulk_apply),
        case(Op::ReachCheck, s, Some(json!({}))),
        case(Op::ReachSettingsGet, &[], None),
        case(Op::ReachSettingsSet, &[], Some(json!({"cfx_list": true}))),
        case(Op::ForwardingSet, s, Some(json!({"enabled": true}))),
        case(Op::ReachFirewallOpen, s, None),
    ]
}

/// Operations the contract test does not run yet, and why. Shrinks as cases are added.
const UNVERIFIED: &[(Op, &str)] = &[
    (Op::TxAdminTicket, "needs a browser session (cookie and secret) and binds a listening port; covered by tests/txgate.rs"),
    (Op::SessionLogout, "204, nothing to read"),
    (Op::SessionLink, "needs a local session"),
    (Op::PanelRestart, "would restart the process"),
    (Op::UserRemove, "204, nothing to read"),
    (Op::ServerUnregister, "204"),
    (Op::ServerDelete, "needs a managed server folder"),
    (Op::ServersAdopt, "needs a real server folder"),
    (Op::ServerOpenFolder, "204"),
    (Op::LogsList, "needs a server folder with logs"),
    (Op::LogsTail, "needs a server folder with logs"),
    (Op::ConsoleStream, "stream"),
    (Op::ConsoleSend, "204"),
    (Op::ConsoleEnable, "204"),
    (Op::ResourceControl, "needs a console helper on a running server"),
    (Op::NodeServerControl, "needs a registered remote node"),
    (Op::NodeAdd, "needs a reachable node"),
    (Op::NodeRemove, "204"),
    (Op::NodeServers, "needs a registered remote node"),
    (Op::WebhookRemove, "204"),
    (Op::WebhookTest, "204"),
    (Op::ScheduleRemove, "204"),
    (Op::InstallSavedClear, "204"),
    (Op::InstallResume, "needs a stopped install (kept in memory by a failed install)"),
    (Op::InstallLog, "needs an install job's log"),
    (Op::SelfUpdateApply, "replaces the program"),
    (Op::UninstallRun, "removes things"),
    (Op::JobsGet, "read by every job case (`wait_job`)"),
    (Op::JobEvents, "stream"),
    (Op::Events, "stream"),
    (Op::AlertsAck, "204"),
    (Op::AlertSettingsGet, "needs a server folder (settings file)"),
    (Op::AlertSettingsSet, "needs a server folder (settings file)"),
    (Op::AssetInspect, "needs an uploaded zip"),
    (Op::AssetImport, "needs an inspected zip"),
    (Op::AssetRemove, "needs an imported asset on disk"),
    (Op::KeyUpdate, "needs a saved key (KeyAdd runs first in its own app)"),
    (Op::KeyRemove, "needs a saved key"),
    (Op::ServerKeySet, "needs a server folder with a server.cfg"),
];

fn app_for(setup: Option<Setup>) -> axum::Router {
    seams();
    let fake = FakeBackend::sample();
    if let Some(f) = setup {
        f(&fake);
    }
    let st = AppState::system(TOKEN.into(), ADDR.parse::<SocketAddr>().unwrap()).with_backend(Arc::new(move || Box::new(fake.clone()))).with_remote(Arc::new(Remote::memory()));
    router(st)
}

fn uri(op: Op, params: &[(&str, &str)], query: &str) -> String {
    let mut path = op.route().1.to_string();
    for (k, v) in params {
        path = path.replace(&format!("{{{k}}}"), v);
    }
    assert!(!path.contains('{'), "{}: a path parameter has no value in the case", op.name());
    let q = if query.is_empty() { String::new() } else { format!("?{query}") };
    format!("/api/v1{path}{q}")
}

async fn send(app: &axum::Router, method: Method, uri: &str, body: Option<&Value>) -> Response<Body> {
    let mut b = Request::builder().method(method.as_str()).uri(uri).header(header::HOST, ADDR).header(header::AUTHORIZATION, format!("Bearer {TOKEN}")).header("x-kora", "1");
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    app.clone().oneshot(b.body(body).unwrap()).await.unwrap()
}

async fn json_of(r: Response<Body>) -> Value {
    let b = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    if b.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&b).unwrap_or_else(|e| panic!("not JSON: {e}: {}", String::from_utf8_lossy(&b)))
    }
}

fn request_ty(sp: &kora_core::api::spec::OpSpec) -> Option<Ty> {
    match &sp.body {
        SpecBody::Json(t) | SpecBody::Either(t, _) => Some(t.clone()),
        _ => None,
    }
}

#[tokio::test]
async fn real_answers_match_the_declared_types() {
    let mut problems = Vec::new();
    let mut checked = Vec::new();
    for c in cases() {
        let app = app_for(c.setup);
        let sp = spec(c.op);
        let name = c.op.name();
        if let (Some(b), Some(t)) = (&c.body, request_ty(&sp)) {
            if let Err(e) = t.check_reads(b) {
                problems.push(format!("{name}: the case's request body: {e}"));
            }
        }
        let r = send(&app, c.op.route().0, &uri(c.op, c.params, c.query), c.body.as_ref()).await;
        let status = r.status().as_u16();
        let v = json_of(r).await;
        if !(200..300).contains(&status) {
            problems.push(format!("{name}: the case answered {status}: {v}"));
            continue;
        }
        match (&sp.response, c.op.is_job()) {
            (Resp::Job, _) | (Resp::Json(_), true) => {
                if status != 202 {
                    problems.push(format!("{name}: a job answers 202, not {status}"));
                }
                let id = v["job"].as_str().unwrap_or_default().to_string();
                if let Resp::Json(t) = &sp.response {
                    if let Err(e) = t.check(&v) {
                        problems.push(format!("{name}: start answer: {e}"));
                    }
                }
                let rec = wait_job(&app, &id).await;
                let jt = sp.job.clone().unwrap_or_default();
                for ev in rec["events"].as_array().cloned().unwrap_or_default() {
                    if let Some(t) = &jt.events {
                        if let Err(e) = t.check(&ev) {
                            problems.push(format!("{name}: job event {ev}: {e}"));
                        }
                    }
                }
                if rec["state"] == "done" {
                    if let Some(t) = &jt.result {
                        if let Err(e) = t.check(&rec["result"]) {
                            problems.push(format!("{name}: job result: {e}"));
                        }
                    }
                } else if rec["state"] == "failed" {
                    problems.push(format!("{name}: the job failed: {}", rec["error"]));
                }
            }
            (Resp::Json(t), false) => {
                if let Err(e) = t.check(&v) {
                    problems.push(format!("{name}: {e}"));
                }
            }
            (Resp::NoContent, _) => {
                if !v.is_null() {
                    problems.push(format!("{name}: declared to answer nothing but answered {v}"));
                }
            }
            (Resp::Stream(..) | Resp::Untyped, _) => {}
        }
        checked.push(name);
    }
    checked.sort_unstable();
    checked.dedup();
    println!("contract: {} operations run against their declared types; unverified: {}", checked.len(), UNVERIFIED.len());
    assert!(problems.is_empty(), "the API does not match its declared types:\n{}", problems.join("\n"));
}

/// The job record (`GET /jobs/{id}`) reads as `JobRecord`, a failed job's error as `JobError`.
async fn wait_job(app: &axum::Router, id: &str) -> Value {
    for _ in 0..400 {
        let r = send(app, Method::Get, &format!("/api/v1/jobs/{id}"), None).await;
        let v = json_of(r).await;
        if v["state"] != "running" {
            let rec = serde_json::from_value::<JobRecord<Value, Value>>(v.clone());
            assert!(rec.is_ok(), "job record: {rec:?}");
            if !v["error"].is_null() {
                let e: Result<JobError, _> = serde_json::from_value(v["error"].clone());
                assert!(e.is_ok(), "job error {}: {e:?}", v["error"]);
            }
            return v;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("job {id} did not finish");
}

/// Every error answer of every operation has the declared envelope.
#[tokio::test]
async fn every_error_answer_has_the_declared_shape() {
    let app = app_for(None);
    let mut bad = Vec::new();
    for op in Op::ALL {
        if matches!(op, Op::ConsoleStream | Op::Events | Op::JobEvents) {
            continue;
        }
        let params: Vec<(String, String)> = kora_core::api::tsgen::path_params(op.route().1).into_iter().map(|p| (p.to_string(), "nope".to_string())).collect();
        let mut path = op.route().1.to_string();
        for (k, v) in &params {
            path = path.replace(&format!("{{{k}}}"), v);
        }
        let body = if matches!(op.route().0, Method::Get | Method::Delete) { None } else { Some(json!({"unexpected": 1})) };
        let r = send(&app, op.route().0, &format!("/api/v1{path}"), body.as_ref()).await;
        if r.status().is_success() {
            continue;
        }
        let status = r.status();
        let ct = r.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
        // Axum's own extractor rejections (a body it cannot read) are plain text, not ours.
        if !ct.starts_with("application/json") {
            continue;
        }
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        if let Err(e) = Ty::of::<ErrorEnvelope>().check(&v) {
            bad.push(format!("{} -> {status}: {e}", op.name()));
        }
    }
    assert!(bad.is_empty(), "error answers that are not the declared envelope:\n{}", bad.join("\n"));
}

#[test]
fn every_operation_is_verified_or_listed_as_unverified() {
    let have: Vec<Op> = cases().iter().map(|c| c.op).collect();
    let skip: Vec<Op> = UNVERIFIED.iter().map(|(o, _)| *o).collect();
    let mut missing = Vec::new();
    for op in Op::ALL {
        let typed = {
            let s = spec(*op);
            matches!(s.body, SpecBody::Json(_) | SpecBody::Raw(_) | SpecBody::Either(..))
                || matches!(s.response, Resp::Json(_) | Resp::Stream(..))
                || s.job.as_ref().is_some_and(|j| j.events.is_some() || j.result.is_some())
        };
        // Untyped operations have nothing to verify yet; the generator lists them.
        if typed && !have.contains(op) && !skip.contains(op) {
            missing.push(op.name());
        }
        assert!(!(have.contains(op) && skip.contains(op)), "{} is both verified and listed as unverified", op.name());
    }
    assert!(missing.is_empty(), "typed operations with neither a contract case nor an UNVERIFIED entry: {missing:?}");
}
