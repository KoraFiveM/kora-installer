// TypeScript mirror of the Kora Panel API (docs/panel-plan.md section 2). JSON is snake_case and
// enums are snake_case strings. Where the plan was ambiguous, the assumption is noted in web/README.md.

export type Mode = 'production' | 'development'
export type ServerStatus = 'running' | 'stopped' | 'missing'

export type Role = 'viewer' | 'operator' | 'admin'

/** Who is signed in. `provider` is how they proved it: the launch link (`local`), Tailscale, or a user token. */
export interface Identity {
  name: string
  provider: 'local' | 'tailscale' | 'token'
  role: Role
}

/** `POST /session`: the cookie is set; `secret` must come back with every request (`X-Kora-Session`). */
export interface SessionCreated {
  ok: boolean
  identity: Identity
  secret: string
}

export interface Meta {
  version: string
  api: number
  platform: 'windows' | 'linux'
  ops: string[]
  node: string
  /** null when not signed in; absent on older servers (treated as a local admin). */
  identity?: Identity | null
  /** The request came from another computer. */
  remote?: boolean
  /** Remote sessions may use the destructive actions (delete, uninstall, console input, install, ...). */
  allow_remote_destructive?: boolean
}

export interface PanelUser {
  name: string
  role: Role
  created: number
}

/** `GET /remote` (local sessions only). Token hashes are never sent. */
export interface RemoteSettings {
  enabled: boolean
  enabled_by_flag: boolean
  bind: string
  port: number
  /** Where Tailscale devices open the panel, while it also listens there (`kora panel --tailscale`). */
  tailnet_url: string | null
  tailscale: boolean
  tailscale_users: Record<string, Role>
  allowed_hosts: string[]
  allow_remote_destructive: boolean
  users: PanelUser[]
  saved_to: string | null
}

export type RemoteSettingsUpdate = Partial<Pick<RemoteSettings, 'enabled' | 'tailscale' | 'tailscale_users' | 'allowed_hosts' | 'allow_remote_destructive'>>

/** `POST /users` answer: the token is shown this once. */
export interface NewUser {
  name: string
  role: Role
  token: string
}

export interface SystemInfo {
  admin: boolean
  platform: 'windows' | 'linux'
  helper_installed: boolean
  headless: boolean
  version: string
  /** True when the browser is on the same machine as Kora (enables "Open folder"). */
  local?: boolean
  /** Same as `admin` (the backend sends both). */
  elevated?: boolean
  kora_home?: string
  /** True when the panel can restart itself ("Restart now" after a self-update). */
  can_restart?: boolean
}

export interface Ports {
  game: number
  txadmin: number
}

export type BadgeKind = 'up_to_date' | 'updates' | 'needs_attention' | 'new_release'
export interface Badge {
  kind: BadgeKind
  count?: number
}

export interface ServerSummary {
  name: string
  dir: string
  mode: Mode
  ports: Ports
  created_unix: number
  status: ServerStatus
  badge: Badge | null
  /** false = adopted (an existing server Kora did not install). */
  managed: boolean
  /**
   * `<this computer's network address>:<game port>`: what players on the same network type to join. Null when
   * Kora could not find the address. It is not an internet address (that is the router's, which Kora cannot see).
   */
  connect?: string | null
}

export interface ServerDetail extends ServerSummary {
  running: boolean
  game_listening: boolean
  txadmin_up: boolean
  console_enabled: boolean
}

export type ControlAction = 'start' | 'stop' | 'restart'

// ---- jobs -----------------------------------------------------------------------------------

export type JobState = 'running' | 'done' | 'failed'

// ---- problems -------------------------------------------------------------------------------

/**
 * Something the user can do about a problem (`kora_core::problem::Action`). Data only: what each
 * kind does is decided by the page (see `lib/problemActions.ts`). A kind this page does not know
 * is not shown.
 */
export type ProblemAction =
  | { kind: 'retry' }
  | { kind: 'retry_step'; step: string }
  | { kind: 'skip_step'; step: string }
  | { kind: 'change_answers'; section?: string }
  | { kind: 'sign_in_github'; scopes?: string[] }
  | { kind: 'install_tool'; tool: 'git' | 'gh' | 'mariadb' | 'unknown' }
  | { kind: 'request_admin' }
  | { kind: 'continue_without_admin' }
  | { kind: 'show_command'; command: string[]; elevated?: boolean }
  | { kind: 'open_link'; url: string }
  | { kind: 'open_log'; job?: string; path?: string }
  | { kind: 'copy_details' }
  | { kind: 'start_database' }

export type ProblemActionKind = ProblemAction['kind']

/** An error a person can act on (`kora_core::problem::Problem`): what API and job errors carry. */
export interface Problem {
  /** Stable machine code, e.g. `github.gh_missing`, `install.step_failed`, `failed`. */
  code: string
  message: string
  /** Technical details for "Show details". */
  details?: string
  /** What the user can do, best first. Empty or missing: the page's generic choices. */
  actions?: ProblemAction[]
}

export interface JobError extends Partial<Omit<Problem, 'message'>> {
  message: string
  command?: string[]
  /** Install failures: what was kept on disk. */
  kept?: string[]
  log_path?: string
  /** Install failures: the step that failed (absent: it stopped before the first step). */
  failed_step?: string
  /** Install failures: `POST /installs/{job}/resume` can carry on with the same answers. */
  resume?: boolean
}

export interface Job<R = unknown, E = unknown> {
  id: string
  op: string
  server: string | null
  state: JobState
  events: E[]
  result: R | null
  error: JobError | null
}

export interface JobStarted {
  job: string
}

// ---- install --------------------------------------------------------------------------------

export type StepStatus = 'pending' | 'running' | 'done' | 'skipped' | 'failed'

export interface ProgressEvent {
  step: string
  title: string
  status: StepStatus
  detail: string
  percent: number
}

export type DbChoice = 'use_existing' | 'install_if_missing' | 'skip'
export type ResourceSelection = 'all' | { only: string[] }

export interface DbAdmin {
  host: string
  port: number
  user: string
  password?: string
}

export interface InstallRequest {
  mode: Mode
  name: string
  dir: string
  hostname: string
  license_key: string | null
  ports: Ports
  txadmin_public: boolean
  artifact_url: string | null
  game_build: number | null
  resources: ResourceSelection
  db: DbChoice
  service: boolean
  firewall: boolean
  github_owner: string | null
  dry_run: boolean
  db_admin: DbAdmin | null
  start: boolean
  /** Cfx.re account that becomes the txAdmin admin, e.g. `fivem:4465268`. Null = a generated local admin. */
  cfx_identifier: string | null
}

export interface InstallOutcome {
  dir: string
  connect: string
  txadmin_url: string
  notes: string[]
  name: string
  start_attempted: boolean
  running: boolean
  txadmin_up: boolean
  game_listening: boolean
  launch_problem: string | null
  log_tail: string[]
  admin_user: string | null
  admin_password_file: string | null
}

export interface InstallDefaults {
  name: string
  dir: string
  hostname: string
  ports: Ports
  /** Answers saved from an install that stopped part-way (never contains secrets). */
  saved: SavedInstall | null
}
export type SavedInstall = Partial<Omit<InstallRequest, 'license_key' | 'db_admin'>>

export interface PortLine {
  port: number
  ok: boolean
  text: string
  suggestion: number | null
}
export interface PortsCheck {
  game: PortLine
  txadmin: PortLine
}
export interface DiskCheck {
  ok: boolean
  free_bytes: number
  needed_bytes: number
  message: string
}
export interface DbLoginCheck {
  ok: boolean
  message: string
  /** A failed login on a database that is installed but stopped: carries the "Start it for me" button. */
  problem?: Problem | null
}
/** Answer of `POST /checks/cfx` (HTTP 200 whether or not the account was found). */
export interface CfxCheck {
  ok: boolean
  id: number | null
  username: string | null
  identifier: string | null
  message: string
}
export interface InstallPlan {
  steps: { id: string; title: string }[]
  /** Plain-language list of what needs administrator rights. */
  needs_admin: string[]
  notes: string[]
}

export interface Artifact {
  build: number
  url: string
  date?: string
}
export interface ArtifactList {
  recommended: number | null
  items: Artifact[]
}

// ---- catalog --------------------------------------------------------------------------------

export type Group =
  | 'pre'
  | 'kora'
  | 'standalone'
  | 'cfx-default'
  | 'default-maps'
  | 'compat'
  | 'props'
  | 'bridges'
  | 'creators'
  | 'other'
export type Kind = 'kora' | 'dependency' | 'optional' | 'internal'

export interface CatalogRelease {
  version: string
  tag: string
  commit: string
}
export interface CatalogResource {
  id: string
  title?: string
  repository: string
  path: string
  group: Group
  kind: Kind
  branch: string
  description?: string
  latest?: string
  versions: CatalogRelease[]
  depends_on: string[]
  creator?: string
  dev_only?: boolean
}
export interface Creator {
  id: string
  name: string
  url?: string
  description?: string
}
export interface Catalog {
  schema_version: number
  resources: CatalogResource[]
  creators: Creator[]
}
export interface ServerCatalog {
  catalog: Catalog
  installed: string[]
  note: string | null
}

export interface PlanItem {
  id: string
  rel: string
  repo: string
  branch: string
  commit: string | null
  fork: boolean
  third_party: boolean
  subpath: string | null
  dependency: boolean
  repository: string
}
export interface AddPlan {
  items: PlanItem[]
  already: string[]
  ensure: string[]
}
export interface AddOutcome {
  installed: string[]
  failed: [string, string][]
  ensured: string[]
  notes: string[]
}

// ---- updates --------------------------------------------------------------------------------

export type ChangeStatus = 'up_to_date' | 'clean' | 'dirty' | 'diverged'
export interface ResourceChange {
  id: string
  from: string
  to: string
  behind: number
  changelog: string[]
  needs_restart: boolean
  status: ChangeStatus
}
export interface UpdatePreview {
  resources: ResourceChange[]
  new_release: string | null
  /** Warnings about the check as a whole (built-in release list used, resources no longer in the catalog). */
  notes?: string[]
}
export type RestartPolicy = 'none' | 'ensure' | 'full_restart' | 'when_quiet'
export type Scope = 'all' | { resources: string[] }

export interface UpdateReport {
  server: string
  updated: { id: string; from: string; to: string }[]
  skipped: [string, string][]
  deferred: [string, string][]
  rolled_back: [string, string][]
  sql_flagged: string[]
  needs_attention: [string, string][]
  forks_synced: string[]
  forks_left_alone: [string, string][]
  /** Resources the run never got to because an earlier one failed (nothing changed in them). */
  not_attempted?: string[]
  /** Automatic updates: versions held back because they were rolled back before. */
  held?: [string, string][]
  notes?: string[]
}
export type UpdateEvent =
  | { type: 'resource'; id: string; state: string }
  | { type: 'snapshot'; id: string }
  | { type: 'health'; message: string }
  | { type: 'progress'; message: string }
  | { type: 'done' }

export interface SnapshotInfo {
  id: string
  created_unix: number
  resources: string[]
}

// ---- schedule / notify / github -------------------------------------------------------------

export type SchedulePreset = 'daily' | 'weekly' | 'custom'
export type RunAs = 'installing_user' | 'system'
export interface ScheduleConfig {
  preset: SchedulePreset
  time: string
  weekday: number | null
  run_as: RunAs
  enabled: boolean
}
export interface ScheduleState {
  schedule: ScheduleConfig | null
  description: string | null
  /** Plain-language trade-off text for the "run as" choice. */
  run_as_help?: string
}

export interface WebhookState {
  configured: boolean
  /** Same as `configured` (the backend sends both). */
  set?: boolean
  redacted: string | null
}

export interface GithubStatus {
  signed_in: boolean
  user: string | null
  /** Missing on older panels (then only `signed_in` is known). */
  state?: 'ready' | 'gh_missing' | 'signed_out' | 'missing_scopes'
  missing_scopes?: string[]
  /** What to do when GitHub is not ready (null when it is). */
  problem?: Problem | null
}
export interface GithubOwner {
  login: string
  kind: 'user' | 'org'
  can_fork?: boolean
  reason?: string | null
}
export interface GithubLoginStart {
  job: string
  code: string
  url: string
}
export interface PrPlan {
  resource: string
  upstream: string
  base: string
  head_owner: string
  head_branch: string
  ahead: number
  dir: string
}

// ---- logs / console / misc ------------------------------------------------------------------

export interface LogFile {
  name: string
  size: number
  modified_unix: number
}

export interface ScanCandidate {
  name: string
  dir: string
  mode: Mode
  kind?: string
  ports?: Ports
}

export interface SelfUpdateInfo {
  current: string
  latest: string | null
  available: boolean
  /** Same as `available` (the backend sends both). */
  update_available?: boolean
  versions: string[]
}
export interface AppStatus {
  /** The OLD machine-wide Kora app (Program Files, /opt/kora) is installed. */
  installed: boolean
  version: string | null
  path: string | null
  desktop_shortcut: boolean
  supported: boolean
  /** The old app, or an old administrator shortcut, is still here: offer "Tidy up old shortcuts". */
  legacy: boolean
  /** What tidying up would remove, one plain sentence each. */
  legacy_items: string[]
  /** The per-user shortcut that opens the panel is in place. */
  shortcuts: boolean
}

export type UninstallItemKind = 'system' | 'folder' | 'database' | 'app'
export interface UninstallItem {
  /** `<server>/service|firewall|timer|account|program|registry|folder|database`, or `app`. */
  item: string
  description: string
  kind: UninstallItemKind
  remove: boolean
}
export interface UninstallPlan {
  items: UninstallItem[]
}

export interface ServerEvent {
  name: string
  status: ServerStatus
  badge: Badge | null
}

// ---- monitoring and alerts ----------------------------------------------------------------

export type MetricsRange = '1h' | '6h' | '24h'

/** One look at a server; absent numbers could not be measured (stopped server, no access to the process). */
export interface MetricSample {
  /** Unix seconds. */
  t: number
  up: boolean
  players?: number
  max_players?: number
  /** Percent of the whole machine's CPU. */
  cpu?: number
  /** Megabytes used by the server and what it started. */
  ram_mb?: number
  /** Percent of this machine's memory in use. */
  sys_ram_pct?: number
  disk_free_mb?: number
  disk_free_pct?: number
  game_up: boolean
  txadmin_up: boolean
}
export interface Metrics {
  samples: MetricSample[]
  latest: MetricSample | null
}

export type AlertKind = 'stopped' | 'recovered' | 'disk_low' | 'ram_high' | 'update_attention'
export interface PanelAlert {
  id: number
  server: string
  kind: AlertKind
  tone: 'danger' | 'warning' | 'success'
  title: string
  message: string
  raised: number
  /** When the problem went away; null while it is still there. */
  resolved: number | null
  acked: boolean
}

export interface AlertRules {
  stopped: boolean
  recovered: boolean
  disk: boolean
  ram: boolean
  updates: boolean
}
export interface AlertSettings {
  enabled: boolean
  rules: AlertRules
  disk_min_gb: number
  disk_min_percent: number
  ram_max_percent: number
  /** Also post to the server's Discord webhook (Notifications tab). */
  discord: boolean
  /** The address itself is never returned. */
  webhook: { configured: boolean; redacted: string | null }
  email: { supported: boolean }
}
export interface AlertSettingsUpdate {
  enabled?: boolean
  rules?: Partial<AlertRules>
  disk_min_gb?: number
  disk_min_percent?: number
  ram_max_percent?: number
  discord?: boolean
  /** Omit to keep the address; '' removes it. */
  webhook_url?: string
}

// ---- hub (fleet) ----------------------------------------------------------------------------

export interface HubNode {
  id: string
  name: string
  url: string
  /** The token is never returned. */
  token_set: boolean
  /** Plain http:// to a non-private, non-Tailscale address. */
  insecure: boolean
  warning: string | null
  reachable: boolean
  version: string | null
  api: number | null
  platform: string | null
  ops: string[]
  server_count: number | null
  can_control: boolean
  last_error: string | null
}

/** A server in the combined list: `node_id` is null for this panel's own servers. */
export interface FleetServer extends ServerSummary {
  node_id: string | null
  node_name: string
  /** Only what the node supports; empty when it is too old to be controlled from here. */
  actions: ControlAction[]
}

export interface Fleet {
  servers: FleetServer[]
  nodes: HubNode[]
  /** True when some node could not be reached. */
  partial: boolean
}

// ---- paid assets (Cfx.re Portal) -------------------------------------------------------------

export type AssetProblemKind =
  | 'not_a_zip'
  | 'no_resources'
  | 'no_fxmanifest'
  | 'missing_fxap'
  | 'unsafe_path'
  | 'skipped_entry'
  | 'too_big'
  | 'zip_bomb'
  | 'encrypted'
  | 'name_taken'
  | 'bad_name'
  | 'duplicate'
  | 'no_folder'

export interface AssetProblem {
  kind: AssetProblemKind
  resource: string | null
  message: string
  /** Stops the import; otherwise a warning. */
  blocking: boolean
}

/** One folder found in a zip. Only the ones with `has_fxmanifest` are imported. */
export interface FoundAsset {
  name: string
  zip_path: string
  has_fxmanifest: boolean
  has_fxap: boolean
  escrowed: boolean
  /** Unpacked size in bytes. */
  size: number
  files: number
  version: string | null
  /** An imported copy is replaced (and kept as a backup). */
  replaces: boolean
}

export interface AssetPlan {
  zip_name: string
  resources: FoundAsset[]
  problems: AssetProblem[]
  total_size: number
  can_import: boolean
}

/** `upload` (an uploaded zip, null when refused) or `download` (a zip from the Downloads folder). */
export interface AssetInspectResult {
  plan: AssetPlan
  upload?: string | null
  download?: string | null
}

export type AssetSource = { upload: string } | { download: string }

export interface PaidAsset {
  name: string
  /** Relative to resources/. */
  path: string
  version: string | null
  imported_unix: number | null
  present: boolean
  has_fxap: boolean
  backups: number
}

export type AssetHintKind = 'entitlement' | 'verify_failed' | 'download_failed'

export interface AssetHint {
  kind: AssetHintKind
  resource: string | null
  /** The console line it was found in. */
  line: string
  fix: string
}

export interface PaidAssets {
  assets: PaidAsset[]
  hints: AssetHint[]
  portal_url: string
}

export interface DownloadZip {
  name: string
  size: number
  modified_unix: number
}

export interface RecentDownloads {
  found: boolean
  folder?: string
  zips: DownloadZip[]
}

export interface ImportedAsset {
  name: string
  version: string | null
  replaced: boolean
  backup: string | null
}

export interface AssetImportReport {
  zip_name: string
  imported: ImportedAsset[]
  ensured: string[]
  notes: string[]
}

export type AssetImportEvent = { type: 'status'; message: string } | { type: 'asset'; name: string; state: 'imported' }

/** One tool of `GET /tools`: Git or the GitHub CLI, from this computer or Kora's own private copy. */
export interface ToolStatus {
  tool: 'git' | 'gh'
  name: string
  label: string
  installed: boolean
  version: string | null
  source: 'system' | 'private' | null
  path: string | null
  /** Found, but it did not run. */
  broken: boolean
  /** Kora can download its own copy on this computer. */
  can_install: boolean
}

export type CheckState = 'ok' | 'problem' | 'info'

/** One line of the System check (`GET /system/check`). */
export interface CheckItem {
  id: 'git' | 'gh' | 'github' | 'database' | 'admin' | 'service'
  label: string
  state: CheckState
  summary: string
  /** What to do about it, with the fix buttons (null when fine). */
  problem: Problem | null
}

export interface SystemCheck {
  items: CheckItem[]
  /** No line needs fixing. */
  ok: boolean
}

/** Progress of an install-tool job (`message`, `fraction` 0..1). */
export interface ToolInstallEvent {
  type: 'progress' | 'status'
  message: string
  fraction?: number
}
