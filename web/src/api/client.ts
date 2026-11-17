import type * as T from './types'
import { ROUTES, type OpName, type OpTypes } from './generated/routes'

export interface ApiErrorBody {
  code: string
  message: string
  command?: string[]
  /** Technical details (a Problem's `details`). */
  details?: string
  /** What the user can do (a Problem's `actions`). */
  actions?: T.ProblemAction[]
}

/** An error answered by the API (or a network failure, code `network`). `message` is for end users. */
export class ApiError extends Error {
  readonly status: number
  readonly code: string
  readonly command?: string[]
  readonly details?: string
  readonly actions?: T.ProblemAction[]
  /** Send the request that failed again (`request_admin`: once more, now asking Windows for permission). */
  replay?: () => Promise<unknown>
  constructor(status: number, body: ApiErrorBody) {
    super(body.message)
    this.name = 'ApiError'
    this.status = status
    this.code = body.code
    this.command = body.command
    this.details = body.details
    this.actions = body.actions
  }
  get needsElevation(): boolean {
    return this.code === 'needs_elevation'
  }
  get unauthorized(): boolean {
    return this.status === 401 || this.code === 'unauthorized'
  }
}

const BASE = '/api/v1'

// The session secret from sign-in. The cookie alone is not enough (browsers also send it to other
// services on this host, such as txAdmin), so every request carries this too. localStorage is per
// origin, so per port: reloads and new tabs keep working, and two panels never mix theirs up.
const SECRET_KEY = 'kora_session_secret'
// Only used when the browser refuses localStorage (some private modes): then it lasts for this page.
let memorySecret: string | null = null
let storageBroken = false

export function sessionSecret(): string | null {
  if (storageBroken) return memorySecret
  try {
    return window.localStorage.getItem(SECRET_KEY)
  } catch {
    return memorySecret
  }
}

export function setSessionSecret(secret: string | null | undefined): void {
  memorySecret = secret ?? null
  try {
    if (secret) window.localStorage.setItem(SECRET_KEY, secret)
    else window.localStorage.removeItem(SECRET_KEY)
    storageBroken = false
  } catch {
    storageBroken = true
  }
}

/** A stream URL (EventSource cannot send headers): the secret goes in `?ks=`. */
export function streamUrl(path: string): string {
  const secret = sessionSecret()
  return secret ? `${BASE}${path}?ks=${encodeURIComponent(secret)}` : `${BASE}${path}`
}

/**
 * Asked when the panel says a sensitive action needs a fresh passkey or one-time-code check
 * (`remote.reauth_required`): the app shows the check and answers whether it succeeded; the request is then sent again once.
 */
type ReauthHandler = () => Promise<boolean>
let onReauth: ReauthHandler | null = null
export function setReauthHandler(fn: ReauthHandler | null): void {
  onReauth = fn
}

type UnauthorizedHandler = () => void
let onUnauthorized: UnauthorizedHandler | null = null
/** The app registers one handler that shows the "session expired" screen. */
export function setUnauthorizedHandler(fn: UnauthorizedHandler | null): void {
  onUnauthorized = fn
}

export function enc(s: string): string {
  return encodeURIComponent(s)
}

async function readError(res: Response): Promise<ApiError> {
  let body: ApiErrorBody | null = null
  try {
    const j = (await res.json()) as { error?: Partial<ApiErrorBody> }
    if (j?.error?.message) body = { code: j.error.code ?? 'internal', message: j.error.message, command: j.error.command, details: j.error.details, actions: Array.isArray(j.error.actions) ? j.error.actions : undefined }
  } catch {
    /* not JSON */
  }
  return new ApiError(res.status, body ?? { code: res.status === 401 ? 'unauthorized' : 'internal', message: `The server answered with an unexpected error (${res.status}).` })
}

export interface RequestOptions {
  method?: string
  body?: unknown
  query?: Record<string, string | number | undefined>
  signal?: AbortSignal
  /** Do not trigger the global 401 handler (used by the sign-in call itself). */
  quiet401?: boolean
  /** Send this file as the body instead of JSON (streamed by the browser). */
  raw?: { body: Blob; type: string; headers?: Record<string, string> }
  /** The person chose "Ask for administrator permission": Windows may show its prompt even for a remote session. */
  askAdmin?: boolean
  /** Do not ask for a fresh passkey check and retry (set on the retry itself). */
  noReauth?: boolean
}

/** Header of [`RequestOptions.askAdmin`] (`kora_web::elevated::ASK_HEADER`). */
export const ADMIN_HEADER = 'X-Kora-Admin'

let askAdminAt = 0
/**
 * The next change sent within a minute asks Windows for administrator permission (the app-wide
 * `request_admin` button arms this, then repeats what failed). Used once.
 */
export function askForAdminNext(): void {
  askAdminAt = Date.now()
}

export async function request<R>(path: string, opts: RequestOptions = {}): Promise<R> {
  const method = opts.method ?? 'GET'
  const headers: Record<string, string> = { Accept: 'application/json' }
  if (method !== 'GET') headers['X-Kora'] = '1'
  const secret = sessionSecret()
  if (secret) headers['X-Kora-Session'] = secret
  if (method !== 'GET' && (opts.askAdmin || (askAdminAt && Date.now() - askAdminAt < 60_000))) {
    headers[ADMIN_HEADER] = 'ask'
    askAdminAt = 0
  }
  let body: BodyInit | undefined
  if (opts.raw) {
    headers['Content-Type'] = opts.raw.type
    Object.assign(headers, opts.raw.headers)
    body = opts.raw.body
  } else if (opts.body !== undefined) {
    headers['Content-Type'] = 'application/json'
    body = JSON.stringify(opts.body)
  }
  let url = BASE + path
  if (opts.query) {
    const q = new URLSearchParams()
    for (const [k, v] of Object.entries(opts.query)) if (v !== undefined) q.set(k, String(v))
    const s = q.toString()
    if (s) url += `?${s}`
  }
  let res: Response
  try {
    res = await fetch(url, { method, headers, body, credentials: 'same-origin', signal: opts.signal })
  } catch (e) {
    if (e instanceof DOMException && e.name === 'AbortError') throw e
    throw new ApiError(0, { code: 'network', message: 'Could not reach Kora. Check that it is still running, then try again.' })
  }
  if (!res.ok) {
    const err = await readError(res)
    err.replay = () => request<R>(path, opts)
    if (err.code === 'remote.reauth_required' && onReauth && !opts.noReauth && (await onReauth())) return request<R>(path, { ...opts, noReauth: true })
    // The session ended: forget its secret (in every tab, it is shared). Not for a failed sign-in (a stale
    // link opened in a new tab must not sign the other tabs out), nor when another tab has signed in
    // since this request left.
    // (The public sign-in routes answer 401 for a wrong name, code or an old link: that says nothing about a session.)
    const signingIn = path === '/session' || path === '/auth/login' || path === '/auth/enrol'
    if (res.status === 401 && !signingIn && sessionSecret() === secret) setSessionSecret(null)
    if (err.unauthorized && !opts.quiet401) onUnauthorized?.()
    throw err
  }
  if (res.status === 204) return undefined as R
  const text = await res.text()
  return (text ? JSON.parse(text) : undefined) as R
}

// ---- calls by operation name: paths, methods and shapes come from the generated route map -----------

/** An `Untyped` (unknown) type: the Rust side has not described it yet. */
type IsUntyped<X> = unknown extends X ? true : false
type Params<K extends OpName> = OpTypes[K]['params']
type Query<K extends OpName> = OpTypes[K]['query']
type BodyOf<K extends OpName> = OpTypes[K]['body']

/** What a call needs: the path parameters, the query and the body the operation declares. */
export type CallInput<K extends OpName> = (Params<K> extends Record<string, never> ? { params?: undefined } : { params: Params<K> }) &
  (Query<K> extends undefined ? { query?: undefined } : { query?: Query<K> }) &
  (BodyOf<K> extends undefined ? { body?: undefined } : IsUntyped<BodyOf<K>> extends true ? { body?: unknown } : { body?: BodyOf<K> })

/** Operations whose response the Rust side has not described yet. Shrinks as they are typed. */
export type UntypedResponseOps = { [K in OpName]: IsUntyped<OpTypes[K]['response']> extends true ? K : never }[OpName]

export interface CallOptions {
  signal?: AbortSignal
  quiet401?: boolean
  /** Do not ask for a fresh passkey check and retry (the check's own calls). */
  noReauth?: boolean
  raw?: RequestOptions['raw']
  askAdmin?: boolean
}

/** `/servers/{s}/logs/{file}` with its parameters filled in (and escaped). */
export function routePath<K extends OpName>(op: K, params?: Params<K>): string {
  const p = (params ?? {}) as Record<string, string>
  return ROUTES[op].path.replace(/\{(\w+)\}/g, (_, k: string) => enc(p[k] ?? ''))
}

function send(op: OpName, input: CallInput<OpName>, opts: CallOptions): Promise<unknown> {
  const r = ROUTES[op]
  return request(routePath(op, input.params as never), { method: r.method, body: input.body, query: input.query as RequestOptions['query'], ...opts })
}

/** Call an operation; the answer has the type the Rust side declares for it. */
export function call<K extends OpName>(op: K, input: CallInput<K>, opts: CallOptions = {}): Promise<OpTypes[K]['response']> {
  return send(op, input as CallInput<OpName>, opts) as Promise<OpTypes[K]['response']>
}

/**
 * Call an operation whose answer is still `Untyped` on the Rust side, with a hand-written type `R`:
 * `legacy<T.Meta>()('meta', {})`. It stops compiling the day the operation is typed in Rust (use
 * `call` then). Every use is a TODO of the api-codegen work (see `UNTYPED_OPS` in `generated/routes.ts`).
 */
export const legacy =
  <R>() =>
  <K extends UntypedResponseOps>(op: K, input: CallInput<K>, opts: CallOptions = {}): Promise<R> =>
    send(op, input as CallInput<OpName>, opts) as Promise<R>

/** The URL of a stream operation (`?ks=` carries the session secret: EventSource cannot send headers). */
export function streamOf<K extends OpName>(op: K, params?: Params<K>): string {
  return streamUrl(routePath(op, params))
}

async function signInWith(body: { token?: string; code?: string }): Promise<void> {
  const r = await call('session_create', { body }, { quiet401: true })
  setSessionSecret((r as T.SessionCreated | undefined)?.secret)
}

/** A sign-in step that finished: remember the session secret that comes with it. */
function keepSecret<R extends { step: string }>(r: R): R {
  const secret = (r as { secret?: string }).secret
  if (secret) setSessionSecret(secret)
  return r
}

/** One function per operation (the route table is generated: `generated/routes.ts`). */
export const api = {
  meta: () => call('meta', {}),
  session: {
    /** Sign in with a pasted token (the launch token here, a user token from another computer). */
    create: (token: string) => signInWith({ token }),
    /** Sign in with the one-time code from a sign-in link (`/#code=...`). */
    createByCode: (code: string) => signInWith({ code }),
    /** Sign in with no token: a Tailscale identity, when the panel is set up for it. */
    createByIdentity: () => signInWith({}),
    logout: () => call('session_logout', {}).finally(() => setSessionSecret(null)),
  },
  remote: {
    get: () => call('remote_get', {}),
    set: (body: T.RemoteSettingsUpdate) => call('remote_set', { body }),
  },
  users: {
    list: () => call('users_list', {}).then((r) => r.users),
    /** Invite a person: they have no token; the answer holds the one-time enrolment link (shown once). */
    add: (name: string, role: T.Role) => call('user_add', { body: { name, role } }),
    remove: (name: string) => call('user_remove', { params: { name } }),
    credentials: (name: string) => call('user_credentials', { params: { name } }),
    reset: (name: string, what: T.ResetWhat) => call('user_reset', { params: { name }, body: { what } }),
  },
  /** API tokens for scripts (never for signing in). */
  apiTokens: {
    list: () => call('api_tokens_list', {}).then((r) => r.tokens),
    create: (body: T.ApiTokenRequest) => call('api_token_create', { body }),
    revoke: (id: number) => call('api_token_revoke', { params: { id: String(id) } }),
  },
  /** Remote sign-in: passkeys, one-time codes, enrolment links, your own credentials. */
  auth: {
    options: () => call('auth_options', {}, { quiet401: true }),
    login: (body: T.LoginRequest) => call('auth_login', { body }, { quiet401: true }).then(keepSecret),
    enrol: (body: T.EnrolRequest) => call('auth_enrol', { body }, { quiet401: true }).then(keepSecret),
    account: () => call('auth_account', {}),
    change: (body: T.AccountChange) => call('auth_account_change', { body }),
    reauth: (body: T.ReauthRequest) => call('auth_reauth', { body }, { noReauth: true }),
  },
  tls: {
    status: () => call('tls_status', {}),
    renew: () => call('tls_renew', {}),
    ca: () => call('tls_ca', {}),
  },
  system: () => call('system_info', {}),
  panel: {
    restart: () => call('panel_restart', {}),
  },
  servers: {
    list: () => call('servers_list', {}).then((r) => r.servers),
    get: (name: string) => call('server_get', { params: { s: name } }),
    control: (name: string, action: T.ControlAction) => call('server_control', { params: { s: name }, body: { action } }),
    delete: (name: string, body: { confirm_name: string; keep_files: boolean; keep_db: boolean; db_admin_password?: string }) => call('server_delete', { params: { s: name }, body }),
    unregister: (name: string) => call('server_unregister', { params: { s: name } }),
    scan: () => call('servers_scan', {}),
    adopt: (dir: string) => call('servers_adopt', { body: { dir } }),
    openFolder: (name: string) => call('server_open_folder', { params: { s: name } }),
  },
  logs: {
    list: (name: string) => call('logs_list', { params: { s: name } }).then((r) => r.files),
    tail: (name: string, file: string, lines = 300) => call('logs_tail', { params: { s: name, file }, query: { lines } }),
  },
  console: {
    streamUrl: (name: string) => streamOf('console_stream', { s: name }),
    send: (name: string, command: string) => call('console_send', { params: { s: name }, body: { command } }),
    enable: (name: string) => call('console_enable', { params: { s: name } }),
  },
  resources: {
    list: (name: string) => call('resources_list', { params: { s: name } }).then((r) => r.resources),
    ensure: (name: string, body: { name: string; restart: boolean }) => call('resource_ensure', { params: { s: name }, body }),
    /** Start, stop or restart one resource on a running server. */
    control: (name: string, resource: string, action: T.ControlAction) => call('resource_control', { params: { s: name, name: resource }, body: { action } }),
    plan: (name: string, ids: string[]) => call('resources_plan', { params: { s: name }, body: { ids } }),
    add: (name: string, ids: string[]) => call('resources_add', { params: { s: name }, body: { ids } }),
    catalog: (name: string) => call('catalog_for_server', { params: { s: name } }),
  },
  assets: {
    list: (name: string) => call('assets_list', { params: { s: name } }),
    /** Upload a zip (the file itself is the body) and get what importing it would do. */
    inspectUpload: (name: string, file: Blob, fileName: string) =>
      call('asset_inspect', { params: { s: name } }, { raw: { body: file, type: 'application/zip', headers: { 'X-File-Name': enc(fileName) } } }),
    /** A recent zip from the Downloads folder of the computer that runs Kora (local sessions only). */
    inspectDownload: (name: string, download: string) => call('asset_inspect', { params: { s: name }, body: { download } }),
    import: (name: string, source: T.AssetSource, replace = true) => call('asset_import', { params: { s: name }, body: { ...source, replace } }),
    remove: (name: string, asset: string) => call('asset_remove', { params: { s: name, name: asset } }),
    downloads: () => call('asset_downloads', {}),
  },
  update: {
    preview: (name: string, scope: T.Scope = 'all') => call('update_preview', { params: { s: name }, body: { scope } }),
    /** `targets`: the preview's planned commit per resource, so exactly what was shown is applied. */
    apply: (name: string, body: { scope: T.Scope; restart: T.RestartPolicy; skip: string[]; targets?: Record<string, string> }) => call('update_apply', { params: { s: name }, body }),
    auto: (name: string) => call('update_auto', { params: { s: name } }),
  },
  snapshots: {
    list: (name: string) => call('snapshots_list', { params: { s: name } }).then((r) => r.snapshots),
    /** `force`: roll back even resources with uncommitted changes (they are thrown away). */
    rollback: (name: string, id: string, force = false) => call('snapshot_rollback', { params: { s: name, id }, body: force ? { force: true } : undefined }),
  },
  webhook: {
    get: (name: string) => call('webhook_get', { params: { s: name } }),
    set: (name: string, url: string) => call('webhook_set', { params: { s: name }, body: { url } }),
    remove: (name: string) => call('webhook_remove', { params: { s: name } }),
    test: (name: string) => call('webhook_test', { params: { s: name } }),
  },
  schedule: {
    get: (name: string) => call('schedule_get', { params: { s: name } }),
    set: (name: string, cfg: T.ScheduleConfig) => call('schedule_set', { params: { s: name }, body: cfg }),
    remove: (name: string) => call('schedule_remove', { params: { s: name } }),
  },
  reach: {
    /** `once`: allow a lookup for this check only (nothing is saved). */
    check: (name: string, once?: T.ReachConsentPatch) => call('reach_check', { params: { s: name }, body: once ?? {} }),
    settings: () => call('reach_settings_get', {}),
    setSettings: (c: T.ReachConsentPatch) => call('reach_settings_set', { body: c }),
    setForwarding: (name: string, enabled: boolean) => call('forwarding_set', { params: { s: name }, body: { enabled } }),
    openFirewall: (name: string) => call('reach_firewall_open', { params: { s: name } }),
    txadminTicket: (name: string) => call('txadmin_ticket', { params: { s: name } }),
  },
  ports: {
    set: (name: string, ports: T.Ports) => call('ports_set', { params: { s: name }, body: ports }),
  },
  feedback: {
    report: (name: string, body: { resource: string; title: string; text: string }) => call('issue_report', { params: { s: name }, body }),
    prPlan: (name: string, resource: string) => call('pr_plan', { params: { s: name }, body: { resource } }),
    prOpen: (name: string, body: { plan: T.PrPlan; title: string; text: string }) => call('pr_open', { params: { s: name }, body }),
  },
  branches: {
    /** Where each resource is (nothing is downloaded). Development servers only. */
    list: (name: string) => call('branches_list', { params: { s: name } }).then((r) => r.resources),
    /** Branches and kept-aside changes of one resource; `refresh` downloads the newest lists from GitHub first. */
    choices: (name: string, resource: string, refresh = false) => call('branch_choices', { params: { s: name, name: resource }, query: refresh ? { refresh: '1' } : undefined }),
    /** Open pull requests of the original repository and of the person's fork. */
    prs: (name: string, resource: string) => call('pr_list', { params: { s: name, name: resource } }),
    /** Switch one resource: another branch, a pull request, or back to the normal version. */
    switch: (name: string, resource: string, body: T.SwitchRequest) => call('branch_switch', { params: { s: name, name: resource }, body }),
    /** Bring kept-aside changes back. */
    restore: (name: string, resource: string, index: number) => call('stash_restore', { params: { s: name, name: resource }, body: { index } }),
    /** Which resources have a branch (a job; result is a `BulkPreview`). */
    bulkPreview: (name: string, branch: string, fetch = true) => call('branch_bulk_preview', { params: { s: name }, body: { branch, fetch } }),
    /** Switch every resource that has the branch, or all back to the normal version (a job; result is a `BulkResult`). */
    bulkApply: (name: string, body: { action: 'branch'; branch: string; resources?: string[]; changes?: T.ChangesChoice } | { action: 'default'; resources?: string[]; changes?: T.ChangesChoice }) =>
      call('branch_bulk_apply', { params: { s: name }, body }),
  },
  github: {
    status: () => call('github_status', {}),
    login: () => call('github_login', {}),
    owners: () => call('github_owners', {}).then((r) => r.owners),
  },
  /** Git and the GitHub CLI, the System check, and starting the database. */
  tools: {
    status: () => call('tools_status', {}).then((r) => r.tools),
    /** Download and set up Kora's own copy (a job; `git` on Linux is the system package and needs administrator rights). */
    install: (tool: 'git' | 'gh') => call('tool_install', { params: { tool } }),
  },
  systemCheck: {
    get: () => call('system_check', {}),
    startDatabase: () => call('db_start', {}),
  },
  /** The audit log (administrators only). `before` is the id of the oldest row already shown. */
  audit: {
    list: (q: { limit?: number; before?: number; server?: string; op?: string; actor?: string; result?: string } = {}) =>
      call('audit_list', { query: { limit: q.limit, before: q.before, server: q.server || undefined, op: q.op || undefined, actor: q.actor || undefined, result: q.result || undefined } }),
  },
  keys: {
    list: () => call('keys_list', {}),
    add: (name: string, license_key: string) => call('key_add', { body: { name, license_key } }),
    /** Rename, put a new key under the name, or both. */
    update: (name: string, body: { new_name?: string; license_key?: string }) => call('key_update', { params: { name }, body }),
    remove: (name: string) => call('key_remove', { params: { name } }),
    /** Use the system keychain (and move the saved keys in), or keep every key in Kora's private file. */
    keychain: (use_keychain: boolean) => call('keychain_set', { body: { use_keychain } }),
    /** Change the licence key of an installed server: a saved key by name, or a new one (optionally saved). */
    setForServer: (server: string, body: { name: string } | { license_key: string; save_as?: string }) => call('server_key_set', { params: { s: server }, body }),
  },
  profile: {
    get: () => call('profile_get', {}),
    /** `null` forgets a field; a field left out stays as it is. */
    set: (body: Partial<T.Profile>) => call('profile_set', { body }),
  },
  checks: {
    cfx: (username: string) => call('check_cfx', { body: { username } }),
  },
  catalog: () => call('catalog_get', {}),
  artifacts: () => call('artifacts_list', {}),
  installs: {
    defaults: () => call('install_defaults', {}),
    clearSaved: () => call('install_saved_clear', {}),
    checkPorts: (ports: T.Ports) => call('check_ports', { body: ports }),
    checkDisk: (dir: string) => call('check_disk', { body: { dir } }),
    checkDbLogin: (admin: T.DbAdmin | null) => call('check_db_login', { body: { admin } }),
    plan: (req: T.InstallRequest) => call('install_plan', { body: req }),
    /** `askAdmin`: the person chose "Ask for administrator permission" (Windows may show its prompt). */
    start: (req: T.InstallRequest, opts: { askAdmin?: boolean } = {}) => call('install_start', { body: req }, { askAdmin: opts.askAdmin }),
    /**
     * Carry on with a stopped install (its job error has `resume: true`): retry the failed step, or skip it.
     * `withoutAdmin`: "Continue without the administrator parts"; `askAdmin`: ask Windows for permission again.
     */
    resume: (job: string, skip?: string, opts: { askAdmin?: boolean; withoutAdmin?: boolean } = {}) =>
      call('install_resume', { params: { job }, body: { ...(skip ? { skip } : {}), ...(opts.withoutAdmin ? { without_admin: true } : {}) } }, { askAdmin: opts.askAdmin }),
    log: (job: string) => call('install_log', { params: { job } }).then((r) => r.lines),
  },
  selfUpdate: {
    check: () => call('self_update_check', {}),
    apply: (version?: string) => call('self_update_apply', { body: version ? { version } : {} }),
  },
  app: {
    status: () => call('app_status', {}),
    /** Make the per-user shortcuts, then remove the old machine-wide copy ("Tidy up old shortcuts"). */
    install: (body: { desktop_shortcut: boolean; dry_run: boolean }) => call('app_install', { body }),
    uninstall: (dryRun = false) => call('app_uninstall', { body: { dry_run: dryRun } }),
  },
  uninstall: {
    plan: () => call('uninstall_plan', {}),
    run: (body: { answers: { item: string; remove: boolean }[]; delete_files: boolean; delete_databases: boolean; only: string[] }) => call('uninstall_run', { body }),
  },
  jobs: {
    /** Recent jobs, also from before the panel last restarted (newest first, without events). */
    list: (limit = 50) => call('jobs_list', { query: { limit } }).then((r) => r.jobs),
    get: <R = unknown, E = unknown>(id: string) => call('jobs_get', { params: { id } }) as Promise<T.Job<R, E>>,
    eventsUrl: (id: string) => streamOf('job_events', { id }),
  },
  eventsUrl: () => streamOf('events'),
  metrics: {
    get: (name: string, range: T.MetricsRange) => call('metrics_get', { params: { s: name }, query: { range } }),
  },
  alerts: {
    list: () => call('alerts_list', {}).then((r) => r.alerts),
    ack: (id: number) => call('alerts_ack', { params: { id: String(id) } }),
    settings: (name: string) => call('alert_settings_get', { params: { s: name } }),
    setSettings: (name: string, body: T.AlertSettingsUpdate) => call('alert_settings_set', { params: { s: name }, body }),
  },
  hub: {
    nodes: () => call('nodes_list', {}).then((r) => r.nodes),
    addNode: (body: { name: string; url: string; token: string }) => call('node_add', { body }).then((r) => r.node),
    removeNode: (id: string) => call('node_remove', { params: { id } }),
    nodeServers: (id: string) => call('node_servers', { params: { id } }).then((r) => r.servers),
    nodeControl: (id: string, server: string, action: T.ControlAction) => call('node_server_control', { params: { id, s: server }, body: { action } }),
    fleet: () => call('fleet', {}),
  },
}

export type Api = typeof api
