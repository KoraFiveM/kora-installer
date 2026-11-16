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
}

export async function request<R>(path: string, opts: RequestOptions = {}): Promise<R> {
  const method = opts.method ?? 'GET'
  const headers: Record<string, string> = { Accept: 'application/json' }
  if (method !== 'GET') headers['X-Kora'] = '1'
  let body: string | undefined
  if (opts.body !== undefined) {
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

/** One function per row of the route table in docs/panel-plan.md section 2.1. */
export const api = {
  meta: () => get<T.Meta>('/meta'),
  session: {
    create: (token: string) => request<void>('/session', { method: 'POST', body: { token }, quiet401: true }),
    /** Sign in with no token: a Tailscale identity, when the panel is set up for it. */
    createByIdentity: () => request<void>('/session', { method: 'POST', body: {}, quiet401: true }),
    logout: () => post<void>('/session/logout'),
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
    delete: (name: string, body: { confirm_name: string; keep_files: boolean; keep_db: boolean }) => del<T.JobStarted>(s(name), body),
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
    streamUrl: (name: string) => `${BASE}${s(name)}/console/stream`,
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
  update: {
    preview: (name: string, scope: T.Scope = 'all') => post<T.JobStarted>(`${s(name)}/update/preview`, { scope }),
    apply: (name: string, body: { scope: T.Scope; restart: T.RestartPolicy; skip: string[] }) => post<T.JobStarted>(`${s(name)}/update/apply`, body),
    auto: (name: string) => post<T.JobStarted>(`${s(name)}/update/auto`),
  },
  snapshots: {
    list: (name: string) => get<{ snapshots: T.SnapshotInfo[] }>(`${s(name)}/snapshots`).then((r) => r.snapshots),
    rollback: (name: string, id: string) => post<T.JobStarted>(`${s(name)}/snapshots/${enc(id)}/rollback`),
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
    eventsUrl: (id: string) => `${BASE}/jobs/${enc(id)}/events`,
  },
  eventsUrl: () => `${BASE}/events`,
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
