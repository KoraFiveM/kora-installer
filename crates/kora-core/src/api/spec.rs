//! What each operation takes and answers: the typed half of the operations table.
//!
//! [`Op::route`](super::Op::route) says WHERE an operation is (method and path); [`spec`] says WHAT
//! goes in and out: the query, the JSON request body, the JSON response, and, for a job, the types
//! of its events and result. Together they are the single description of the HTTP API. The web
//! panel's TypeScript (`web/src/api/generated/`) is generated from them by [`super::tsgen`], and CI
//! fails when the checked-in files are out of date.
//!
//! Only built with the `ts` feature (the web crate's tests turn it on; a release build never does).
//! The `match` in [`spec`] has no wildcard arm: a new [`Op`] does not pass `cargo test` until it has
//! a line here. `OpSpec::untyped()` is an honest placeholder: the generated client then says
//! `Untyped` (= `unknown`) for that operation, and the generator reports it.
//!
//! The types named here must derive `Serialize`, `Deserialize` and `ts_rs::TS` (behind
//! `cfg_attr(feature = "ts", ...)`). The panel's contract test (`crates/kora-web/tests/api_contract.rs`)
//! calls the real routes and checks that what they answer reads as the type declared here and writes
//! back to the same JSON, so a handler that drifts from its declared type fails a test.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

use super::tsgen::Collector;
use super::Op;

/// A Rust type that appears in the API, as seen by the generator and the contract test.
#[derive(Clone)]
pub struct Ty {
    /// The TypeScript name, e.g. `ServerDetail` or `Array<ServerSummary>`.
    pub name: String,
    pub(crate) collect: fn(&mut Collector),
    /// `Ok` when the JSON reads as the type and writes back unchanged.
    pub(crate) check: fn(&Value) -> Result<(), String>,
}

impl Ty {
    pub fn of<T: TS + Serialize + DeserializeOwned + 'static>() -> Ty {
        Ty { name: T::name(&super::tsgen::config()), collect: |c| c.add::<T>(), check: check_roundtrip::<T> }
    }

    /// Does `json` read as this type and write back as the same JSON?
    pub fn check(&self, json: &Value) -> Result<(), String> {
        (self.check)(json)
    }
}

impl std::fmt::Debug for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

fn check_roundtrip<T: Serialize + DeserializeOwned>(json: &Value) -> Result<(), String> {
    let t: T = serde_json::from_value(json.clone()).map_err(|e| format!("does not read as {}: {e}", std::any::type_name::<T>()))?;
    let back = serde_json::to_value(&t).map_err(|e| e.to_string())?;
    if &back == json {
        Ok(())
    } else {
        Err(format!("reads as {} but writes back differently (a field the type does not know, or null versus left out):\n  answered: {json}\n  written:  {back}", std::any::type_name::<T>()))
    }
}

/// The request body.
#[derive(Debug, Clone)]
pub enum Body {
    /// No body.
    None,
    Json(Ty),
    /// The file itself, with this content type (asset upload).
    Raw(&'static str),
    /// Not described yet: the generated client says `Untyped`.
    Untyped,
}

/// The successful answer.
#[derive(Debug, Clone)]
pub enum Resp {
    /// `204`, or an empty `200`.
    NoContent,
    Json(Ty),
    /// `202 {"job": id}`: see [`OpSpec::job`].
    Job,
    /// `text/event-stream` whose `data:` lines are JSON of this type.
    Stream(Ty),
    /// Not described yet: the generated client says `Untyped`.
    Untyped,
}

/// What a job reports: its events (`GET /jobs/{id}/events`, and `events` of `GET /jobs/{id}`) and its result.
#[derive(Debug, Clone, Default)]
pub struct JobTypes {
    pub events: Option<Ty>,
    pub result: Option<Ty>,
}

#[derive(Debug, Clone)]
pub struct OpSpec {
    pub query: Option<Ty>,
    pub body: Body,
    pub response: Resp,
    pub job: Option<JobTypes>,
}

impl OpSpec {
    /// Not described yet (an explicit gap, listed by the generator).
    pub fn untyped() -> Self {
        OpSpec { query: None, body: Body::Untyped, response: Resp::Untyped, job: None }
    }
    /// A job whose request body is not described yet.
    pub fn untyped_job() -> Self {
        OpSpec { query: None, body: Body::Untyped, response: Resp::Job, job: Some(JobTypes::default()) }
    }
    /// No body in, JSON out.
    pub fn get<R: TS + Serialize + DeserializeOwned + 'static>() -> Self {
        OpSpec { query: None, body: Body::None, response: Resp::Json(Ty::of::<R>()), job: None }
    }
    /// JSON in, JSON out.
    pub fn call<B: TS + Serialize + DeserializeOwned + 'static, R: TS + Serialize + DeserializeOwned + 'static>() -> Self {
        OpSpec { query: None, body: Body::Json(Ty::of::<B>()), response: Resp::Json(Ty::of::<R>()), job: None }
    }
    /// JSON in, nothing out.
    pub fn send<B: TS + Serialize + DeserializeOwned + 'static>() -> Self {
        OpSpec { query: None, body: Body::Json(Ty::of::<B>()), response: Resp::NoContent, job: None }
    }
    /// Nothing in, nothing out.
    pub fn nothing() -> Self {
        OpSpec { query: None, body: Body::None, response: Resp::NoContent, job: None }
    }
    /// Nothing in, a job out.
    pub fn start_job() -> Self {
        OpSpec { query: None, body: Body::None, response: Resp::Job, job: Some(JobTypes::default()) }
    }
    /// JSON in, a job out.
    pub fn start_job_with<B: TS + Serialize + DeserializeOwned + 'static>() -> Self {
        OpSpec { query: None, body: Body::Json(Ty::of::<B>()), response: Resp::Job, job: Some(JobTypes::default()) }
    }
    pub fn with_body<B: TS + Serialize + DeserializeOwned + 'static>(mut self) -> Self {
        self.body = Body::Json(Ty::of::<B>());
        self
    }
    pub fn with_query<Q: TS + Serialize + DeserializeOwned + 'static>(mut self) -> Self {
        self.query = Some(Ty::of::<Q>());
        self
    }
    /// The type of the job's events (a job op only).
    pub fn events<E: TS + Serialize + DeserializeOwned + 'static>(mut self) -> Self {
        self.job.get_or_insert_with(JobTypes::default).events = Some(Ty::of::<E>());
        self
    }
    /// The type of the job's result (a job op only).
    pub fn result<R: TS + Serialize + DeserializeOwned + 'static>(mut self) -> Self {
        self.job.get_or_insert_with(JobTypes::default).result = Some(Ty::of::<R>());
        self
    }

    pub fn body_typed(&self) -> bool {
        !matches!(self.body, Body::Untyped)
    }
    pub fn response_typed(&self) -> bool {
        !matches!(self.response, Resp::Untyped)
    }
}

/// The types of `op`. No wildcard arm: every operation is listed.
pub fn spec(op: Op) -> OpSpec {
    match op {
        Op::Meta => OpSpec::untyped(),
        Op::SessionCreate => OpSpec::untyped(),
        Op::SystemInfo => OpSpec::untyped(),
        Op::ServersList => OpSpec::untyped(),
        Op::ServerGet => OpSpec::untyped(),
        Op::ServerControl => OpSpec::untyped_job(),
        Op::ServerDelete => OpSpec::untyped_job(),
        Op::ServerUnregister => OpSpec::untyped(),
        Op::ServersScan => OpSpec::untyped_job(),
        Op::ServersAdopt => OpSpec::untyped(),
        Op::ServerOpenFolder => OpSpec::untyped(),
        Op::LogsList => OpSpec::untyped(),
        Op::LogsTail => OpSpec::untyped(),
        Op::ConsoleStream => OpSpec::untyped(),
        Op::ConsoleSend => OpSpec::untyped(),
        Op::ConsoleEnable => OpSpec::untyped(),
        Op::ResourcesList => OpSpec::untyped(),
        Op::ResourceEnsure => OpSpec::untyped(),
        Op::ResourceControl => OpSpec::untyped(),
        Op::UpdatePreview => OpSpec::untyped_job(),
        Op::UpdateApply => OpSpec::untyped_job(),
        Op::UpdateAuto => OpSpec::untyped_job(),
        Op::SnapshotsList => OpSpec::untyped(),
        Op::SnapshotRollback => OpSpec::untyped_job(),
        Op::CatalogForServer => OpSpec::untyped(),
        Op::ResourcesPlan => OpSpec::untyped(),
        Op::ResourcesAdd => OpSpec::untyped_job(),
        Op::WebhookGet => OpSpec::untyped(),
        Op::WebhookSet => OpSpec::untyped(),
        Op::WebhookRemove => OpSpec::untyped(),
        Op::WebhookTest => OpSpec::untyped(),
        Op::ScheduleGet => OpSpec::untyped(),
        Op::ScheduleSet => OpSpec::untyped(),
        Op::ScheduleRemove => OpSpec::untyped(),
        Op::PortsSet => OpSpec::untyped(),
        Op::IssueReport => OpSpec::untyped(),
        Op::PrPlan => OpSpec::untyped(),
        Op::PrOpen => OpSpec::untyped(),
        Op::GithubStatus => OpSpec::untyped(),
        Op::GithubLogin => OpSpec::untyped_job(),
        Op::GithubOwners => OpSpec::untyped(),
        Op::CatalogGet => OpSpec::untyped(),
        Op::ArtifactsList => OpSpec::untyped(),
        Op::InstallDefaults => OpSpec::untyped(),
        Op::InstallSavedClear => OpSpec::untyped(),
        Op::CheckPorts => OpSpec::untyped(),
        Op::CheckDisk => OpSpec::untyped(),
        Op::CheckDbLogin => OpSpec::untyped(),
        Op::InstallPlan => OpSpec::untyped(),
        Op::InstallStart => OpSpec::untyped_job(),
        Op::InstallResume => OpSpec::untyped_job(),
        Op::InstallLog => OpSpec::untyped(),
        Op::SelfUpdateCheck => OpSpec::untyped(),
        Op::SelfUpdateApply => OpSpec::untyped_job(),
        Op::AppStatus => OpSpec::untyped(),
        Op::AppInstall => OpSpec::untyped(),
        Op::AppUninstall => OpSpec::untyped(),
        Op::UninstallPlan => OpSpec::untyped(),
        Op::UninstallRun => OpSpec::untyped_job(),
        Op::JobsGet => OpSpec::untyped(),
        Op::JobEvents => OpSpec::untyped(),
        Op::Events => OpSpec::untyped(),
        Op::MetricsGet => OpSpec::untyped(),
        Op::AlertsList => OpSpec::untyped(),
        Op::AlertsAck => OpSpec::untyped(),
        Op::AlertSettingsGet => OpSpec::untyped(),
        Op::AlertSettingsSet => OpSpec::untyped(),
        Op::NodesList => OpSpec::untyped(),
        Op::NodeAdd => OpSpec::untyped(),
        Op::NodeRemove => OpSpec::untyped(),
        Op::NodeServers => OpSpec::untyped(),
        Op::NodeServerControl => OpSpec::untyped_job(),
        Op::Fleet => OpSpec::untyped(),
        Op::RemoteGet => OpSpec::untyped(),
        Op::RemoteSet => OpSpec::untyped(),
        Op::UsersList => OpSpec::untyped(),
        Op::UserAdd => OpSpec::untyped(),
        Op::UserRemove => OpSpec::untyped(),
        Op::SessionLogout => OpSpec::untyped(),
        Op::SessionLink => OpSpec::untyped(),
        Op::PanelRestart => OpSpec::untyped(),
        Op::CheckCfx => OpSpec::untyped(),
        Op::AssetsList => OpSpec::untyped(),
        Op::AssetInspect => OpSpec::untyped(),
        Op::AssetImport => OpSpec::untyped_job(),
        Op::AssetRemove => OpSpec::untyped(),
        Op::AssetDownloads => OpSpec::untyped(),
        Op::ToolsStatus => OpSpec::untyped(),
        Op::ToolInstall => OpSpec::untyped_job(),
        Op::SystemCheck => OpSpec::untyped(),
        Op::DbStart => OpSpec::untyped_job(),
        Op::KeysList => OpSpec::untyped(),
        Op::KeyAdd => OpSpec::untyped(),
        Op::KeyUpdate => OpSpec::untyped(),
        Op::KeyRemove => OpSpec::untyped(),
        Op::ServerKeySet => OpSpec::untyped(),
        Op::ProfileGet => OpSpec::untyped(),
        Op::ProfileSet => OpSpec::untyped(),
        Op::BranchesList => OpSpec::untyped(),
        Op::BranchChoices => OpSpec::untyped(),
        Op::PrList => OpSpec::untyped(),
        Op::BranchSwitch => OpSpec::untyped(),
        Op::StashRestore => OpSpec::untyped(),
        Op::BranchBulkPreview => OpSpec::untyped_job(),
        Op::BranchBulkApply => OpSpec::untyped_job(),
    }
}
