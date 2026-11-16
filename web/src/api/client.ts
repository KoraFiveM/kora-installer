import type * as T from './types'

export interface ApiErrorBody {
  code: string
  message: string
  command?: string[]
}

/** An error answered by the API (or a network failure, code `network`). `message` is for end users. */
export class ApiError extends Error {
  readonly status: number
  readonly code: string
  readonly command?: string[]
  constructor(status: number, body: ApiErrorBody) {
    super(body.message)
    this.name = 'ApiError'
    this.status = status
    this.code = body.code
    this.command = body.command
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
    if (j?.error?.message) body = { code: j.error.code ?? 'internal', message: j.error.message, command: j.error.command }
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
}

export async function request<R>(path: string, opts: RequestOptions = {}): Promise<R> {
  const method = opts.method ?? 'GET'
  const headers: Record<string, string> = { Accept: 'application/json' }
  if (method !== 'GET') headers['X-Kora'] = '1'
  const secret = sessionSecret()
  if (secret) headers['X-Kora-Session'] = secret
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
    // The session ended: forget its secret (in every tab, it is shared). Not for a failed sign-in (a stale
    // link opened in a new tab must not sign the other tabs out), nor when another tab has signed in
    // since this request left.
    if (res.status === 401 && path !== '/session' && sessionSecret() === secret) setSessionSecret(null)
    if (err.unauthorized && !opts.quiet401) onUnauthorized?.()
    throw err
  }
  if (res.status === 204) return undefined as R
  const text = await res.text()
  return (text ? JSON.parse(text) : undefined) as R
}

const get = <R>(path: string, query?: RequestOptions['query']) => request<R>(path, { query })

const post = <R>(path: string, body?: unknown) => request<R>(path, { method: 'POST', body })
const put = <R>(path: string, body?: unknown) => request<R>(path, { method: 'PUT', body })
const del = <R>(path: string, body?: unknown) => request<R>(path, { method: 'DELETE', body })

const s = (name: string) => `/servers/${enc(name)}`

async function signInWith(body: { token?: string; code?: string }): Promise<void> {
  const r = await request<T.SessionCreated | undefined>('/session', { method: 'POST', body, quiet401: true })
  setSessionSecret(r?.secret)
}

/** One function per row of the route table in docs/panel-plan.md section 2.1. */
export const api = {
  meta: () => get<T.Meta>('/meta'),
  session: {
    /** Sign in with a pasted token (the launch token here, a user token from another computer). */
    create: (token: string) => signInWith({ token }),
    /** Sign in with the one-time code from a sign-in link (`/#code=...`). */
    createByCode: (code: string) => signInWith({ code }),
    /** Sign in with no token: a Tailscale identity, when the panel is set up for it. */
    createByIdentity: () => signInWith({}),
    logout: () => post<void>('/session/logout').finally(() => setSessionSecret(null)),
  },
  remote: {
    get: () => get<T.RemoteSettings>('/remote'),
    set: (body: T.RemoteSettingsUpdate) => put<T.RemoteSettings>('/remote', body),
  },
  users: {
    list: () => get<{ users: T.PanelUser[] }>('/users').then((r) => r.users),
    add: (name: string, role: T.Role) => post<T.NewUser>('/users', { name, role }),
    remove: (name: string) => del<void>(`/users/${enc(name)}`),
  },
  system: () => get<T.SystemInfo>('/system'),
  servers: {
    list: () => get<{ servers: T.ServerSummary[] }>('/servers').then((r) => r.servers),
    get: (name: string) => get<T.ServerDetail>(s(name)),
    control: (name: string, action: T.ControlAction) => post<T.JobStarted>(`${s(name)}/control`, { action }),
    delete: (name: string, body: { confirm_name: string; keep_files: boolean; keep_db: boolean; db_admin_password?: string }) => del<T.JobStarted>(s(name), body),
    unregister: (name: string) => del<void>(`${s(name)}/registration`),
    scan: () => post<T.JobStarted>('/servers/scan'),
    adopt: (dir: string) => post<T.ServerSummary>('/servers/adopt', { dir }),
    openFolder: (name: string) => post<void>(`${s(name)}/open-folder`),
  },
  logs: {
    list: (name: string) => get<{ files: T.LogFile[] }>(`${s(name)}/logs`).then((r) => r.files),
    tail: (name: string, file: string, lines = 300) => get<{ file: string; lines: string[] }>(`${s(name)}/logs/${enc(file)}`, { lines }),
  },
  console: {
    streamUrl: (name: string) => streamUrl(`${s(name)}/console/stream`),
    send: (name: string, command: string) => post<void>(`${s(name)}/console`, { command }),
    enable: (name: string) => put<void>(`${s(name)}/console/enabled`, { enabled: true }),
  },
  resources: {
    list: (name: string) => get<{ resources: string[] }>(`${s(name)}/resources`).then((r) => r.resources),
    ensure: (name: string, body: { name: string; restart: boolean }) => post<{ message: string }>(`${s(name)}/ensure`, body),
    plan: (name: string, ids: string[]) => post<T.AddPlan>(`${s(name)}/resources/plan`, { ids }),
    add: (name: string, ids: string[]) => post<T.JobStarted>(`${s(name)}/resources`, { ids }),
    catalog: (name: string) => get<T.ServerCatalog>(`${s(name)}/catalog`),
  },
  assets: {
    list: (name: string) => get<T.PaidAssets>(`${s(name)}/assets`),
    /** Upload a zip (the file itself is the body) and get what importing it would do. */
    inspectUpload: (name: string, file: Blob, fileName: string) =>
      request<T.AssetInspectResult>(`${s(name)}/assets/inspect`, { method: 'POST', raw: { body: file, type: 'application/zip', headers: { 'X-File-Name': enc(fileName) } } }),
    /** A recent zip from the Downloads folder of the computer that runs Kora (local sessions only). */
    inspectDownload: (name: string, download: string) => post<T.AssetInspectResult>(`${s(name)}/assets/inspect`, { download }),
    import: (name: string, source: T.AssetSource, replace = true) => post<T.JobStarted>(`${s(name)}/assets`, { ...source, replace }),
    remove: (name: string, asset: string) => del<{ notes: string[] }>(`${s(name)}/assets/${enc(asset)}`),
    downloads: () => get<T.RecentDownloads>('/assets/downloads'),
  },
  update: {
    preview: (name: string, scope: T.Scope = 'all') => post<T.JobStarted>(`${s(name)}/update/preview`, { scope }),
    /** `targets`: the preview's planned commit per resource, so exactly what was shown is applied. */
    apply: (name: string, body: { scope: T.Scope; restart: T.RestartPolicy; skip: string[]; targets?: Record<string, string> }) => post<T.JobStarted>(`${s(name)}/update/apply`, body),
    auto: (name: string) => post<T.JobStarted>(`${s(name)}/update/auto`),
  },
  snapshots: {
    list: (name: string) => get<{ snapshots: T.SnapshotInfo[] }>(`${s(name)}/snapshots`).then((r) => r.snapshots),
    /** `force`: roll back even resources with uncommitted changes (they are thrown away). */
    rollback: (name: string, id: string, force = false) => post<T.JobStarted>(`${s(name)}/snapshots/${enc(id)}/rollback`, force ? { force: true } : undefined),
  },
  webhook: {
    get: (name: string) => get<T.WebhookState>(`${s(name)}/webhook`),
    set: (name: string, url: string) => put<T.WebhookState>(`${s(name)}/webhook`, { url }),
    remove: (name: string) => del<void>(`${s(name)}/webhook`),
    test: (name: string) => post<void>(`${s(name)}/webhook/test`),
  },
  schedule: {
    get: (name: string) => get<T.ScheduleState>(`${s(name)}/schedule`),
    set: (name: string, cfg: T.ScheduleConfig) => put<T.ScheduleState>(`${s(name)}/schedule`, cfg),
    remove: (name: string) => del<void>(`${s(name)}/schedule`),
  },
  ports: {
    set: (name: string, ports: T.Ports) => put<{ notes: string[] }>(`${s(name)}/ports`, ports),
  },
  feedback: {
    report: (name: string, body: { resource: string; title: string; text: string }) => post<{ url: string }>(`${s(name)}/issues`, body),
    prPlan: (name: string, resource: string) => post<T.PrPlan>(`${s(name)}/pr/plan`, { resource }),
    prOpen: (name: string, body: { plan: T.PrPlan; title: string; text: string }) => post<{ url: string }>(`${s(name)}/pr`, body),
  },
  github: {
    status: () => get<T.GithubStatus>('/github/status'),
    login: () => post<T.GithubLoginStart>('/github/login'),
    owners: () => get<{ owners: T.GithubOwner[] }>('/github/owners').then((r) => r.owners),
  },
  checks: {
    cfx: (username: string) => post<T.CfxCheck>('/checks/cfx', { username }),
  },
  catalog: () => get<T.Catalog>('/catalog'),
  artifacts: () => get<T.ArtifactList>('/artifacts'),
  installs: {
    defaults: () => get<T.InstallDefaults>('/installs/defaults'),
    clearSaved: () => del<void>('/installs/saved'),
    checkPorts: (ports: T.Ports) => post<T.PortsCheck>('/checks/ports', ports),
    checkDisk: (dir: string) => post<T.DiskCheck>('/checks/disk', { dir }),
    checkDbLogin: (admin: T.DbAdmin | null) => post<T.DbLoginCheck>('/checks/db-login', { admin }),
    plan: (req: T.InstallRequest) => post<T.InstallPlan>('/installs/plan', req),
    start: (req: T.InstallRequest) => post<T.JobStarted>('/installs', req),
    log: (job: string) => get<{ lines: string[] }>(`/installs/${enc(job)}/log`).then((r) => r.lines),
  },
  selfUpdate: {
    check: () => get<T.SelfUpdateInfo>('/self-update'),
    apply: (version?: string) => post<T.JobStarted>('/self-update', version ? { version } : {}),
  },
  app: {
    status: () => get<T.AppStatus>('/app'),
    install: (body: { desktop_shortcut: boolean; force: boolean; dry_run: boolean }) => put<{ message: string }>('/app', body),
    uninstall: (dryRun = false) => del<{ message: string }>('/app', { dry_run: dryRun }),
  },
  uninstall: {
    plan: () => post<T.UninstallPlan>('/uninstall/plan'),
    run: (body: { answers: { item: string; remove: boolean }[]; delete_files: boolean; delete_databases: boolean; only: string[] }) => post<T.JobStarted>('/uninstall', body),
  },
  jobs: {
    get: <R = unknown, E = unknown>(id: string) => get<T.Job<R, E>>(`/jobs/${enc(id)}`),
    eventsUrl: (id: string) => streamUrl(`/jobs/${enc(id)}/events`),
  },
  eventsUrl: () => streamUrl('/events'),
  metrics: {
    get: (name: string, range: T.MetricsRange) => get<T.Metrics>(`${s(name)}/metrics`, { range }),
  },
  alerts: {
    list: () => get<{ alerts: T.PanelAlert[] }>('/alerts').then((r) => r.alerts),
    ack: (id: number) => post<void>(`/alerts/${id}/ack`),
    settings: (name: string) => get<T.AlertSettings>(`${s(name)}/alerts`),
    setSettings: (name: string, body: T.AlertSettingsUpdate) => put<T.AlertSettings>(`${s(name)}/alerts`, body),
  },
  hub: {
    nodes: () => get<{ nodes: T.HubNode[] }>('/nodes').then((r) => r.nodes),
    addNode: (body: { name: string; url: string; token: string }) => post<{ node: T.HubNode }>('/nodes', body).then((r) => r.node),
    removeNode: (id: string) => del<void>(`/nodes/${enc(id)}`),
    nodeServers: (id: string) => get<{ servers: T.FleetServer[] }>(`/nodes/${enc(id)}/servers`).then((r) => r.servers),
    nodeControl: (id: string, server: string, action: T.ControlAction) => post<T.JobStarted>(`/nodes/${enc(id)}/servers/${enc(server)}/control`, { action }),
    fleet: () => get<T.Fleet>('/fleet'),
  },
}

export type Api = typeof api
