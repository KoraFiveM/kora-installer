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
}

export interface ServerDetail extends ServerSummary {
  running: boolean
  game_listening: boolean
  txadmin_up: boolean
  console_enabled: boolean
  /** Full address to connect with, when the server knows it. */
  connect?: string
}

export type ControlAction = 'start' | 'stop' | 'restart'

// ---- jobs -----------------------------------------------------------------------------------

export type JobState = 'running' | 'done' | 'failed'

export interface JobError {
  code?: string
  message: string
  command?: string[]
  /** Install failures: what was kept on disk. */
  kept?: string[]
  log_path?: string
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
  app: { desktop_shortcut: boolean } | null
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
}
export type UpdateEvent =
  | { type: 'resource'; id: string; state: string }
  | { type: 'snapshot'; id: string }
  | { type: 'health'; message: string }
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
  redacted: string | null
}

export interface GithubStatus {
  signed_in: boolean
  user: string | null
}
export interface GithubOwner {
  login: string
  kind: 'user' | 'org'
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
}

export interface SelfUpdateInfo {
  current: string
  latest: string | null
  available: boolean
  versions: string[]
}
export interface AppStatus {
  installed: boolean
  version: string | null
  path: string | null
  desktop_shortcut: boolean
  /** The Kora app only exists on Windows. */
  supported: boolean
}

export type UninstallItemKind = 'system' | 'folder' | 'database' | 'app'
export interface UninstallItem {
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
