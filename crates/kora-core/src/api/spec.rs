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
use super::wire as w;
use super::Op;
use crate::installer::Ports;

/// A Rust type that appears in the API, as seen by the generator and the contract test.
#[derive(Clone)]
pub struct Ty {
    /// The TypeScript name, e.g. `ServerDetail` or `Array<ServerSummary>`.
    pub name: String,
    pub(crate) collect: fn(&mut Collector),
    /// `Ok` when the JSON reads as the type and writes back unchanged.
    pub(crate) check: fn(&Value) -> Result<(), String>,
    /// `Ok` when the JSON reads as the type (for a request body: it may leave out what has a default).
    pub(crate) reads: fn(&Value) -> Result<(), String>,
}

impl Ty {
    pub fn of<T: TS + Serialize + DeserializeOwned + 'static>() -> Ty {
        Ty { name: T::name(&super::tsgen::config()), collect: |c| c.add::<T>(), check: check_roundtrip::<T>, reads: check_reads::<T> }
    }

    /// Does `json` read as this type (a request body only needs to be understood)?
    pub fn check_reads(&self, json: &Value) -> Result<(), String> {
        (self.reads)(json)
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

fn check_reads<T: DeserializeOwned>(json: &Value) -> Result<(), String> {
    serde_json::from_value::<T>(json.clone()).map(|_| ()).map_err(|e| format!("does not read as {}: {e}", std::any::type_name::<T>()))
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
    /// Either this JSON or the file itself with that content type.
    Either(Ty, &'static str),
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
    /// `text/event-stream`: SSE event name and the JSON type of its data.
    Stream(&'static str, Ty),
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
    /// A server-sent-events stream; `event` is the SSE event name whose data is JSON of type `E`.
    pub fn stream<E: TS + Serialize + DeserializeOwned + 'static>(event: &'static str) -> Self {
        OpSpec { query: None, body: Body::None, response: Resp::Stream(event, Ty::of::<E>()), job: None }
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
    /// The body may also be the file itself (a zip), with this content type.
    pub fn or_raw(mut self, content_type: &'static str) -> Self {
        if let Body::Json(t) = &self.body {
            self.body = Body::Either(t.clone(), content_type);
        }
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
        Op::Meta => OpSpec::get::<w::Meta>(),
        Op::SessionCreate => OpSpec::call::<w::SessionRequest, w::SessionCreated>(),
        Op::SystemInfo => OpSpec::get::<w::SystemInfo>(),
        Op::ServersList => OpSpec::get::<w::ServersList>(),
        Op::ServerGet => OpSpec::get::<w::ServerDetail>(),
        Op::ServerControl => OpSpec::start_job_with::<w::ControlRequest>().events::<w::StatusEvent>().result::<w::ControlResult>(),
        Op::ServerDelete => OpSpec::start_job_with::<w::DeleteRequest>().events::<w::StatusEvent>().result::<w::Notes>(),
        Op::ServerUnregister => OpSpec::nothing(),
        Op::ServersScan => OpSpec::start_job().events::<w::NoEvents>().result::<w::ScanResult>(),
        Op::ServersAdopt => OpSpec::call::<w::AdoptRequest, w::ServerSummary>(),
        Op::ServerOpenFolder => OpSpec::nothing(),
        Op::LogsList => OpSpec::get::<w::LogsList>(),
        Op::LogsTail => OpSpec::get::<w::LogTail>().with_query::<w::LogTailQuery>(),
        Op::ConsoleStream => OpSpec::stream::<w::ConsoleLine>("line"),
        Op::ConsoleSend => OpSpec::send::<w::ConsoleCommand>(),
        Op::ConsoleEnable => OpSpec::nothing(),
        Op::ResourcesList => OpSpec::get::<w::ResourcesList>(),
        Op::ResourceEnsure => OpSpec::call::<w::EnsureRequest, w::Message>(),
        Op::ResourceControl => OpSpec::call::<w::ResourceControlRequest, w::Message>(),
        Op::UpdatePreview => OpSpec::start_job_with::<w::UpdatePreviewRequest>().events::<w::UpdateEvent>().result::<w::UpdatePreviewResult>(),
        Op::UpdateApply => OpSpec::start_job_with::<w::UpdateApplyRequest>().events::<w::UpdateEvent>().result::<w::UpdateReport>(),
        Op::UpdateAuto => OpSpec::start_job().events::<w::NoEvents>().result::<w::UpdateReport>(),
        Op::SnapshotsList => OpSpec::get::<w::SnapshotsList>(),
        Op::SnapshotRollback => OpSpec::start_job_with::<w::RollbackRequest>().events::<w::UpdateEvent>().result::<w::Nothing>(),
        Op::CatalogForServer => OpSpec::get::<w::ServerCatalog>(),
        Op::ResourcesPlan => OpSpec::call::<w::IdsRequest, crate::addresources::AddPlan>(),
        Op::ResourcesAdd => OpSpec::start_job_with::<w::IdsRequest>().events::<w::ResourceEvent>().result::<crate::addresources::AddOutcome>(),
        Op::WebhookGet => OpSpec::get::<w::WebhookState>(),
        Op::WebhookSet => OpSpec::call::<w::WebhookSetRequest, w::WebhookState>(),
        Op::WebhookRemove => OpSpec::nothing(),
        Op::WebhookTest => OpSpec::nothing(),
        Op::ScheduleGet => OpSpec::get::<w::ScheduleState>(),
        Op::ScheduleSet => OpSpec::call::<crate::service::ScheduleConfig, w::ScheduleState>(),
        Op::ScheduleRemove => OpSpec::nothing(),
        Op::PortsSet => OpSpec::call::<Ports, w::Notes>(),
        Op::IssueReport => OpSpec::call::<w::IssueRequest, w::CreatedUrl>(),
        Op::PrPlan => OpSpec::call::<w::PrPlanRequest, crate::feedback::PrPlan>(),
        Op::PrOpen => OpSpec::call::<w::PrOpenRequest, w::CreatedUrl>(),
        Op::GithubStatus => OpSpec::get::<w::GithubStatus>(),
        Op::GithubLogin => OpSpec::get::<w::GithubLoginStart>().events::<w::NoEvents>().result::<w::GithubLoginResult>(),
        Op::GithubOwners => OpSpec::get::<w::GithubOwners>(),
        Op::CatalogGet => OpSpec::get::<w::Catalog>(),
        Op::ArtifactsList => OpSpec::get::<w::ArtifactList>(),
        Op::InstallDefaults => OpSpec::get::<w::InstallDefaults>(),
        Op::InstallSavedClear => OpSpec::nothing(),
        Op::CheckPorts => OpSpec::call::<Ports, w::PortsCheck>(),
        Op::CheckDisk => OpSpec::call::<w::DirRequest, w::DiskCheck>(),
        Op::CheckDbLogin => OpSpec::call::<w::DbLoginRequest, w::DbLoginCheck>(),
        Op::InstallPlan => OpSpec::call::<w::InstallRequest, w::InstallPlan>(),
        Op::InstallStart => OpSpec::start_job_with::<w::InstallRequest>().events::<w::InstallEvent>().result::<crate::installer::InstallOutcome>(),
        Op::InstallResume => OpSpec::start_job_with::<w::ResumeRequest>().events::<w::InstallEvent>().result::<crate::installer::InstallOutcome>(),
        Op::InstallLog => OpSpec::get::<w::InstallLog>(),
        Op::SelfUpdateCheck => OpSpec::get::<w::SelfUpdateInfo>(),
        Op::SelfUpdateApply => OpSpec::start_job_with::<w::SelfUpdateRequest>().events::<w::StatusEvent>().result::<w::SelfUpdateResult>(),
        Op::AppStatus => OpSpec::get::<w::AppStatus>(),
        Op::AppInstall => OpSpec::call::<w::AppInstallRequest, w::Message>(),
        Op::AppUninstall => OpSpec::call::<w::AppUninstallRequest, w::Message>(),
        Op::UninstallPlan => OpSpec::get::<w::UninstallPlan>(),
        Op::UninstallRun => OpSpec::start_job_with::<w::UninstallRequest>().events::<w::StatusEvent>().result::<w::UninstallResult>(),
        Op::JobsGet => OpSpec::get::<w::JobRecord>(),
        Op::JobEvents => OpSpec::stream::<w::JobFailedData>("failed"),
        Op::JobsList => OpSpec::get::<w::JobsList>().with_query::<w::JobsQuery>(),
        Op::AuditList => OpSpec::get::<w::AuditPage>().with_query::<w::AuditQuery>(),
        Op::Events => OpSpec::stream::<w::ServersList>("servers"),
        Op::MetricsGet => OpSpec::get::<w::Metrics>().with_query::<w::MetricsQuery>(),
        Op::AlertsList => OpSpec::get::<w::AlertsList>(),
        Op::AlertsAck => OpSpec::nothing(),
        Op::AlertSettingsGet => OpSpec::get::<w::AlertSettings>(),
        Op::AlertSettingsSet => OpSpec::call::<w::AlertSettingsUpdate, w::AlertSettings>(),
        Op::NodesList => OpSpec::get::<w::NodesList>(),
        Op::NodeAdd => OpSpec::call::<w::NodeAddRequest, w::NodeAdded>(),
        Op::NodeRemove => OpSpec::nothing(),
        Op::NodeServers => OpSpec::get::<w::NodeServers>(),
        Op::NodeServerControl => OpSpec::start_job_with::<w::ControlRequest>().events::<w::StatusEvent>().result::<w::ControlResult>(),
        Op::Fleet => OpSpec::get::<w::Fleet>(),
        Op::RemoteGet => OpSpec::get::<w::RemoteSettings>(),
        Op::RemoteSet => OpSpec::call::<w::RemoteSettingsUpdate, w::RemoteSettings>(),
        Op::UsersList => OpSpec::get::<w::UsersList>(),
        Op::UserAdd => OpSpec::call::<w::UserAddRequest, w::NewUser>(),
        Op::UserRemove => OpSpec::nothing(),
        Op::SessionLogout => OpSpec::nothing(),
        Op::SessionLink => OpSpec::get::<w::SessionLink>(),
        Op::PanelRestart => OpSpec::get::<w::PanelRestarting>(),
        Op::CheckCfx => OpSpec::call::<w::CfxRequest, w::CfxCheck>(),
        Op::AssetsList => OpSpec::get::<w::PaidAssets>(),
        Op::AssetInspect => OpSpec::call::<w::AssetInspectRequest, w::AssetInspectResult>().or_raw("application/zip"),
        Op::AssetImport => OpSpec::start_job_with::<w::AssetImportRequest>().events::<w::AssetImportEvent>().result::<crate::paidassets::ImportReport>(),
        Op::AssetRemove => OpSpec::get::<w::Notes>(),
        Op::AssetDownloads => OpSpec::get::<w::RecentDownloads>(),
        Op::ToolsStatus => OpSpec::get::<w::ToolsList>(),
        Op::ToolInstall => OpSpec::start_job().events::<w::ToolInstallEvent>().result::<w::ToolInstallResult>(),
        Op::SystemCheck => OpSpec::get::<w::SystemCheck>(),
        Op::DbStart => OpSpec::start_job().events::<w::StatusEvent>().result::<w::DbStartResult>(),
        Op::KeysList => OpSpec::get::<w::SavedKeys>(),
        Op::KeyAdd => OpSpec::call::<w::KeyAddRequest, w::KeyAdded>(),
        Op::KeyUpdate => OpSpec::call::<w::KeyUpdateRequest, w::KeyChange>(),
        Op::KeyRemove => OpSpec::get::<w::KeyRemoved>(),
        Op::ServerKeySet => OpSpec::call::<w::ServerKeyRequest, w::ServerKeyChanged>(),
        Op::ProfileGet => OpSpec::get::<w::Profile>(),
        Op::ProfileSet => OpSpec::call::<w::ProfileUpdate, w::Profile>(),
        Op::BranchesList => OpSpec::get::<w::BranchesList>(),
        Op::BranchChoices => OpSpec::get::<crate::branches::Choices>().with_query::<w::BranchChoicesQuery>(),
        Op::PrList => OpSpec::get::<crate::branches::PrChoices>(),
        Op::BranchSwitch => OpSpec::call::<w::SwitchRequest, crate::branches::SwitchResult>(),
        Op::StashRestore => OpSpec::call::<w::StashRestoreRequest, crate::branches::SwitchResult>(),
        Op::BranchBulkPreview => OpSpec::start_job_with::<w::BulkPreviewRequest>().events::<w::BulkEvent>().result::<crate::branches::BulkPreview>(),
        Op::BranchBulkApply => OpSpec::start_job_with::<w::BulkApplyRequest>().events::<w::BulkEvent>().result::<crate::branches::BulkResult>(),
    }
}
