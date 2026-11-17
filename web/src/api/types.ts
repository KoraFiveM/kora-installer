// TypeScript mirror of the Kora Panel API (docs/panel-plan.md section 2). JSON is snake_case and
// enums are snake_case strings. Where the plan was ambiguous, the assumption is noted in web/README.md.

// ---- jobs -----------------------------------------------------------------------------------

// ---- problems -------------------------------------------------------------------------------

// ---- install --------------------------------------------------------------------------------

// ---- catalog --------------------------------------------------------------------------------

// ---- updates --------------------------------------------------------------------------------

// ---- schedule / notify / github -------------------------------------------------------------

// ---- logs / console / misc ------------------------------------------------------------------

export interface ServerEvent {
  name: string
  status: ServerStatus
  badge: Badge | null
}

// ---- monitoring and alerts ----------------------------------------------------------------

export type MetricsRange = '1h' | '6h' | '24h'

// ---- hub (fleet) ----------------------------------------------------------------------------

// ---- paid assets (Cfx.re Portal) -------------------------------------------------------------

export type AssetSource = { upload: string } | { download: string }

// ---- saved licence keys and the remembered profile --------------------------------------------

// ---- branches and pull requests (Development servers) ---------------------------------------

// ---- generated from the Rust side (`./generated/types.ts`): re-exported so existing imports keep working ----
// (Do not define a type here that the Rust side already describes: add it to crates/kora-core/src/api/wire/.)

export type {
  ConsentKind,
  Connect as ReachConnect,
  Consent as ReachConsent,
  ForwardStatus,
  ForwardingView,
  Fix as ReachFix,
  FixId as ReachFixId,
  Layer as ReachLayer,
  LayerId as ReachLayerId,
  PublicSource,
  ReachConsentPatch,
  ReachReport,
  ReachState,
  TxAdminTicket,
  AddOutcome,
  AddPlan,
  AdoptRequest,
  AlertEmail,
  AlertKind,
  AlertRules,
  AlertRulesUpdate,
  AlertSettings,
  AlertSettingsUpdate,
  AlertTone,
  AlertWebhook,
  AlertsList,
  AppInstallRequest,
  AppStatus,
  AppUninstallRequest,
  Artifact,
  ArtifactList,
  AssetHint,
  AssetHintKind,
  AssetImportEvent,
  AssetImportReport,
  AssetImportRequest,
  AssetInspectRequest,
  AssetInspectResult,
  AssetPlan,
  AssetProblem,
  AssetProblemKind,
  Badge,
  BadgeKind,
  BranchChoices,
  BranchChoicesQuery,
  BranchLists,
  BranchStash,
  BranchState,
  BranchesList,
  BulkApplyRequest,
  BulkEvent,
  BulkItem,
  BulkPreview,
  BulkPreviewRequest,
  BulkResult,
  BulkRow,
  BulkStatus,
  Catalog,
  CatalogRelease,
  CatalogResource,
  CfxCheck,
  CfxRequest,
  ChangeStatus,
  ChangesChoice,
  CheckItem,
  CheckState,
  ConsoleCommand,
  ConsoleLine,
  ControlAction,
  ControlRequest,
  ControlResult,
  CreatedUrl,
  Creator,
  DbAdmin,
  DbChoice,
  DbLoginCheck,
  DbLoginRequest,
  DbStartResult,
  DeleteRequest,
  DirRequest,
  DiskCheck,
  DownloadZip,
  EnsureRequest,
  ErrorBody,
  ErrorEnvelope,
  Fleet,
  FleetServer,
  FoundAsset,
  GithubLoginResult,
  GithubLoginStart,
  GithubOwner,
  GithubOwners,
  GithubState,
  GithubStatus,
  Group,
  HubNode,
  Identity,
  IdsRequest,
  ImportedAsset,
  InstallDefaults,
  InstallEvent,
  InstallLog,
  InstallOutcome,
  InstallPlan,
  InstallRequest,
  IssueRequest,
  JobError,
  JobSummary,
  JobsList,
  AuditRow,
  AuditPage,
  JobFailedData,
  JobStarted,
  KeyAddRequest,
  KeyAdded,
  KeyChange,
  KeyRemoved,
  KeyUpdateRequest,
  Kind,
  LogFile,
  LogTail,
  LogTailQuery,
  LogsList,
  Message,
  Meta,
  MetricSample,
  Metrics,
  MetricsQuery,
  Mode,
  NewUser,
  NoEvents,
  NodeAddRequest,
  NodeAdded,
  NodeServers,
  NodesList,
  Notes,
  Nothing,
  OwnerKind,
  PaidAsset,
  PaidAssets,
  PanelAlert,
  PanelRestarting,
  PanelUser,
  PlanItem,
  PlanStep,
  PlatformName,
  PortLine,
  Ports,
  PortsCheck,
  PrChoices,
  PrOpenRequest,
  PrPlan,
  PrPlanRequest,
  PrSource,
  Preset,
  Profile,
  ProfileUpdate,
  ProgressEvent,
  Provider,
  PullRequest,
  RecentDownloads,
  Remote,
  RemoteSettings,
  RemoteSettingsUpdate,
  ResourceBranch,
  ResourceChange,
  ResourceControlRequest,
  ResourceEvent,
  ResourceSelection,
  ResourcesList,
  RestartPolicy,
  Resumable,
  ResumeRequest,
  Role,
  RollbackRequest,
  RunAs,
  SampleBadge,
  SavedKey,
  SavedKeyInfo,
  SavedKeys,
  ScanCandidate,
  ScanKind,
  ScanResult,
  ScheduleConfig,
  ScheduleState,
  Scope,
  SelfUpdateInfo,
  SelfUpdateRequest,
  SelfUpdateResult,
  ServerCatalog,
  ServerDetail,
  ServerKeyChanged,
  ServerKeyRequest,
  ServerSummary,
  ServersList,
  SessionCreated,
  SessionLink,
  SessionRequest,
  SnapshotInfo,
  SnapshotsList,
  StashRestoreRequest,
  StatusDetail,
  StatusEvent,
  StepStatus,
  SwitchKind,
  SwitchNow,
  SwitchRequest,
  SwitchResult,
  SwitchStatus,
  SystemCheck,
  SystemInfo,
  Tool,
  ToolInstallEvent,
  ToolInstallResult,
  ToolSource,
  ToolStatus,
  ToolsList,
  UninstallAnswer,
  UninstallItem,
  UninstallItemKind,
  UninstallPlan,
  UninstallRequest,
  UninstallResult,
  UpdateApplyRequest,
  UpdateEvent,
  UpdatePreviewRequest,
  UpdatePreviewResult,
  UpdateReport,
  UpdatedResource,
  UserAddRequest,
  UsersList,
  WebhookSetRequest,
  WebhookState,
} from './generated/types'
import type { Badge, Action, JobRecord, Status, SavedInstall as GeneratedSavedInstall, Problem as GeneratedProblem, JobState as GeneratedJobState, UpdatePreviewResult, Preset } from './generated/types'
export type JobState = GeneratedJobState
/** `running`, `stopped` or `missing` (the folder is gone). */
export type ServerStatus = Status
/**
 * Something the user can do about a problem (`kora_core::problem::Action`). Data only: what each kind does is
 * decided by the page (`lib/problemActions.tsx`). A newer server may send a kind this page does not know
 * (the Rust `Action::Unknown`, or any other string): such an action is not shown.
 */
export type ProblemAction = Action
export type ProblemActionKind = ProblemAction['kind']
/** An error a person can act on (`kora_core::problem::Problem`). */
export type Problem = GeneratedProblem
/** `GET /jobs/{id}`; `R` is the job's result type and `E` its event type. */
export type Job<R = unknown, E = unknown> = JobRecord<R, E>
/** The result of the update preview job (`badge` and `notes` may be missing from an older panel). */
export type UpdatePreview = Omit<UpdatePreviewResult, 'badge' | 'notes'> & Partial<Pick<UpdatePreviewResult, 'badge' | 'notes'>>
/** The answers of a stopped install, as saved (every field may be missing from an older panel). */
export type SavedInstall = Partial<GeneratedSavedInstall>
export type SchedulePreset = Preset

// Backups of a server (generated from the Rust types in kora_core::backup and api::wire::backups).
export type {
  BackupCreateRequest,
  BackupCreated,
  BackupEvent,
  BackupInfo,
  BackupSchedule,
  BackupSettings,
  BackupSummary,
  BackupsOverview,
  BackupsRemoved,
  BackupsView,
  Cadence,
  DatabaseBackup,
  DatabaseNote,
  DbKind,
  PassphraseRequest,
  ResourceBackup,
  ResourceChoice,
  RestorePreview,
  RestoreReport,
  Retention,
  ServerBackupLine,
  UndoInfo,
  VerifyResult,
  Label as BackupLabel,
  BundleCreateRequest,
  BundleFile,
  BundleFileRequest,
  BundleFileText,
  BundleListing,
  BundleSaved,
  Omitted,
  SupportRequest,
  SupportSummary,
} from './generated/types'
