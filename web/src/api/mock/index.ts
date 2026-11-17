// In-app mock backend: replaces window.fetch (for /api/v1) and window.EventSource.
// Only loaded when VITE_MOCK=1 (or `npm run dev`), see src/main.tsx.
import type * as T from '../types'
import type { SavedInstall as GeneratedSavedInstall } from '../generated/types'
import { installMonitorMock } from './monitor'
import { installHubMock } from './hub'
import { installToolchainMock, mockDbStoppedProblem, mockGhInstalled, mockGhMissingProblem, resetToolchainMock, setMockGhInstalled, stopMockDatabase } from './toolchain'
import { installKeysMock } from './keys'
import { installStoreMock } from './store'
import { installBranchesMock } from './branches'
import { installResourceMock } from './resource'
import { installCollabMock } from './collab'
import { installReachMock } from './reach'
import { CONSOLE_LINES, INSTALLED, LOG_FILES, fakeLogLines, seedCatalog, seedServers, seedSnapshots } from './data'
import { behaviourOf, resolveScenario, seeded, type Behaviour, type ScenarioName } from './scenarios'
import { ADMIN_STEP, adminNeeds, promptAnswer, promptProblem, remoteProblem, withoutAdminReq } from './windowsAdmin'

const BASE = '/api/v1'

/** The old `mode` word for an install request (what older Kora versions read). */
const compatMode = (req: T.InstallRequest): T.Mode => (req.versions ? (req.versions === 'latest' ? 'development' : 'production') : req.mode ?? 'production')

// ---- state (rebuilt by `init`, so a scenario starts from a clean world) ------------------------

let scenario: ScenarioName = 'default'
let beh: Behaviour = behaviourOf('default')
let speed = 1 // multiplier for every wait; 0 in tests of the mock
let rand = seeded(1)
let catalog = seedCatalog()
let servers: T.ServerSummary[] = []
let installed: Record<string, string[]> = {}
let snapshots: Record<string, T.SnapshotInfo[]> = {}
let consoleEnabled: Record<string, boolean> = {}
let webhooks: Record<string, string> = {}
let schedules: Record<string, T.ScheduleConfig> = {}
let prefs = { github: false as boolean, savedInstall: null as GeneratedSavedInstall | null, app: false, shortcuts: true, desktopShortcut: false, version: '0.4.0', signedIn: true }
let dirtyPreviewed: Record<string, T.UpdatePreview> = {}
let paidAssets: Record<string, T.PaidAsset[]> = {}
let uploads = new Map<string, string>()
let nextId = 1
let consoleTick = 0

function init(name: ScenarioName) {
  scenario = name
  beh = behaviourOf(name)
  speed = beh.jobSlow
  rand = seeded(7)
  catalog = seedCatalog()
  servers = seedServers(name)
  installed = {}
  snapshots = seedSnapshots()
  consoleEnabled = {}
  for (const s of servers) {
    installed[s.name] = [...(INSTALLED[s.name] ?? ['oxmysql', 'ox_lib', 'kora_core', 'chat'])]
    snapshots[s.name] ??= []
    consoleEnabled[s.name] = beh.trouble ? false : name === 'fleet' ? s.status === 'running' : s.name === 'kora-live'
  }
  webhooks = beh.trouble && servers[0] ? { [servers[0].name]: 'https://discord.com/api/webhooks/123456789012345678/TOKEN-that-no-longer-exists' } : {}
  schedules = {}
  prefs = { github: false, savedInstall: null, app: false, shortcuts: true, desktopShortcut: false, version: '0.4.0', signedIn: true }
  dirtyPreviewed = {}
  paidAssets = {}
  uploads = new Map()
  for (const s of servers.slice(0, 1)) paidAssets[s.name] = [{ name: 'qs-housing', path: '[paid]/qs-housing', version: '3.2.1', imported_unix: Math.floor(Date.now() / 1000) - 3 * 86400, present: true, has_fxap: true, backups: 1 }]
  branchesMock?.()
  collabMock?.reset()
  resourceMock?.reset()
  jobs.clear()
  busy.clear()
  paused.clear()
  keysMock.reset({ trouble: beh.trouble, withKeys: name === 'fleet' })
  reachMock.reset()
  storeMock.reset(name === 'restarted')
  if (name === 'restarted') {
    // The install that was running when the panel stopped, written down with its (saved) licence key by name.
    paused.set('old-3', {
      req: { mode: 'production', name: 'Staging', dir: '/srv/fivem/staging', hostname: 'Staging', license_key_name: 'Main', ports: { game: 30122, txadmin: 40122 }, txadmin_public: false, resources: 'all', db: 'install_if_missing', service: false, firewall: false, dry_run: false, start: true } as unknown as T.InstallRequest,
      failed: '',
      attempt: 1,
      done: [],
      skip: new Set(),
    })
  }
  nextId = 1
  consoleTick = 0
  resetToolchainMock(beh.trouble)
}

class Fail extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly command?: string[],
    readonly extra: Partial<T.JobError> = {},
  ) {
    super(message)
  }
}

const find = (name: string): T.ServerSummary => {
  const s = servers.find((x) => x.name === name)
  if (!s) throw new Fail(404, 'not_found', `There is no server called "${name}".`)
  return s
}
const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms * speed))
const elevate = (what: string, cmd: string[]) =>
  new Fail(403, 'needs_elevation', `${what} needs administrator rights. Run this command as administrator:`, ['kora', ...cmd], {
    actions: [{ kind: 'request_admin' }, { kind: 'show_command', command: ['kora', ...cmd], elevated: true }],
  })

// ---- buses ----------------------------------------------------------------------------------

type Fn = (kind: string, data: unknown) => void
const serverBus = new Set<Fn>()
const consoleBus = new Map<string, Set<Fn>>()
function pushServer(s: T.ServerSummary) {
  const ev: T.ServerEvent = { name: s.name, status: s.status, badge: s.badge }
  serverBus.forEach((f) => f('server', ev))
}
function pushConsole(name: string, text: string) {
  consoleBus.get(name)?.forEach((f) => f('line', { line: text }))
}
let ticker: ReturnType<typeof setInterval> | null = null
function startTicker() {
  if (ticker) return
  ticker = setInterval(() => {
    consoleTick++
    for (const s of servers) if (s.status === 'running') pushConsole(s.name, `[12:${String(Math.floor(consoleTick / 60) % 60).padStart(2, '0')}:${String(consoleTick % 60).padStart(2, '0')}] ${CONSOLE_LINES[(consoleTick + s.name.length) % CONSOLE_LINES.length]}`)
  }, 2500)
}

// ---- jobs -----------------------------------------------------------------------------------

interface MJob {
  id: string
  op: string
  server: string | null
  state: T.JobState
  events: unknown[]
  result: unknown
  error: T.JobError | null
  started: number
  ended: number | null
  subs: Set<Fn>
}
const jobs = new Map<string, MJob>()
const busy = new Set<string>()

function startJob(op: string, server: string | null, run: (emit: (e: unknown) => void) => Promise<unknown>, opts: { exclusive?: boolean } = {}): T.JobStarted {
  if (server && opts.exclusive !== false) {
    if (busy.has(server)) throw new Fail(409, 'conflict', `Another job is already running on "${server}". Wait for it to finish and try again.`)
    busy.add(server)
  }
  const job: MJob = { id: `job-${nextId++}`, op, server, state: 'running', events: [], result: null, error: null, started: Math.floor(Date.now() / 1000), ended: null, subs: new Set() }
  jobs.set(job.id, job)
  const emit = (e: unknown) => {
    job.events.push(e)
    job.subs.forEach((f) => f('progress', e))
  }
  void (async () => {
    await sleep(60)
    try {
      job.result = await run(emit)
      job.state = 'done'
      job.ended = Math.floor(Date.now() / 1000)
    } catch (e) {
      job.state = 'failed'
      job.ended = Math.floor(Date.now() / 1000)
      job.error = e instanceof Fail ? { code: e.code, message: e.message, command: e.command, ...e.extra } : { code: 'internal', message: e instanceof Error ? e.message : 'Something went wrong.' }
    }
    if (server) busy.delete(server)
    job.subs.forEach((f) => f(job.state === 'done' ? 'done' : 'failed', {}))
  })()
  return { job: job.id }
}

const jobView = (j: MJob): T.Job => ({ id: j.id, op: j.op, server: j.server, state: j.state, events: j.events, result: j.result, error: j.error, started: j.started, ended: j.ended })

// ---- helpers --------------------------------------------------------------------------------

const detail = (s: T.ServerSummary): T.ServerDetail => ({
  ...s,
  connect: `192.168.1.20:${s.ports.game}`,
  status_detail: { running: s.status === 'running', game_listening: s.status === 'running', txadmin_up: s.status === 'running' },
  running: s.status === 'running',
  game_listening: s.status === 'running',
  txadmin_up: s.status === 'running',
  console_enabled: !!consoleEnabled[s.name],
})

function portOwner(port: number, except?: string): string | null {
  if (port === 3306) return 'the database'
  for (const s of servers) {
    if (s.name === except) continue
    if (s.ports.game === port || s.ports.txadmin === port) return s.name
  }
  return null
}
function nextFree(port: number, taken: number[]): number {
  let p = port
  while (portOwner(p) || taken.includes(p)) p++
  return p
}
function portLine(port: number, taken: number[], except?: string): T.PortLine {
  const owner = portOwner(port, except)
  if (!owner) return { port, ok: true, text: `${port} is free`, suggestion: null }
  const sug = nextFree(port + 1, taken)
  return { port, ok: false, text: `${port} is already in use${owner === 'the database' ? '' : ` by "${owner}"`}. ${sug} is free.`, suggestion: sug }
}

function badgeFrom(p: T.UpdatePreview): T.Badge {
  const clean = p.resources.filter((r) => r.status === 'clean').length
  if (p.resources.some((r) => r.status === 'dirty' || r.status === 'diverged')) return { kind: 'needs_attention' }
  if (clean > 0) return { kind: 'updates', count: clean }
  if (p.new_release) return { kind: 'new_release' }
  return { kind: 'up_to_date' }
}

function makePreview(s: T.ServerSummary): T.UpdatePreview {
  const r = (id: string, behind: number, status: T.ChangeStatus, changelog: string[] = [], needs_restart = false): T.ResourceChange => ({
    id,
    from: status === 'up_to_date' ? 'c4c4c4c' : `${id.length}a1b2c3`.slice(0, 7),
    to: status === 'up_to_date' ? 'c4c4c4c' : '9b7e0d4',
    behind,
    changelog,
    needs_restart,
    status,
  })
  if (beh.trouble) {
    return {
      new_release: null,
      resources: [
        r('kora_core', 4, 'clean', ['Fix payroll rounding', 'Faster character loading'], true),
        r('kora_inventory', 1, 'clean', ['Stack limits are now per item']),
        r('kora_phone', 2, 'dirty', ['New contacts screen', 'Fix call audio'], true),
        r('kora_jobs', 3, 'diverged', ['Duty roster rewrite']),
      ],
    }
  }
  if (s.name === 'kora-dev') {
    return {
      new_release: null,
      resources: [
        { id: 'kora_core', from: '3f2a91c', to: '9b7e0d4', behind: 4, changelog: ['Fix payroll rounding', 'Faster character loading', 'Add admin duty toggle', 'Update translations'], needs_restart: true, status: 'clean' },
        { id: 'kora_inventory', from: '11c0aa2', to: '5d8e3f1', behind: 1, changelog: ['Stack limits are now per item'], needs_restart: false, status: 'clean' },
        { id: 'kora_phone', from: '7a7a7a7', to: '8c8c8c8', behind: 2, changelog: ['New contacts screen', 'Fix call audio'], needs_restart: true, status: 'dirty' },
        { id: 'kora_jobs', from: 'c4c4c4c', to: 'c4c4c4c', behind: 0, changelog: [], needs_restart: false, status: 'up_to_date' },
      ],
    }
  }
  const k = s.badge?.kind
  if (k === 'updates') return { new_release: null, resources: Array.from({ length: Math.min(s.badge?.count ?? 1, 6) }, (_, i) => r(['kora_core', 'kora_inventory', 'kora_jobs', 'kora_phone', 'chat', 'mapmanager'][i], 1 + i, 'clean', ['A small fix'])) }
  if (k === 'needs_attention') return { new_release: null, resources: [r('kora_core', 2, 'dirty', ['A fix']), r('kora_jobs', 0, 'up_to_date')] }
  if (k === 'new_release') return { new_release: '1.5.0', resources: [r('kora_core', 0, 'up_to_date')] }
  return { new_release: null, resources: s.status === 'missing' ? [] : [r('kora_core', 0, 'up_to_date')] }
}

function checkPort(n: unknown): number {
  const v = Number(n)
  if (!Number.isInteger(v) || v < 1024 || v > 65535) throw new Fail(400, 'bad_request', 'Ports must be whole numbers from 1024 to 65535.')
  return v
}

// ---- install --------------------------------------------------------------------------------

const INSTALL_STEPS: [string, string][] = [
  ['prepare.checks', 'Checking your computer'],
  ['prepare.git', 'Checking Git is available'],
  ['build.fxserver', 'Downloading the FiveM server'],
  ['build.verify', 'Verifying the download'],
  ['build.db', 'Setting up the database'],
  ['build.resources', 'Installing resources'],
  ['build.config', 'Writing the server settings'],
  ['build.firewall', 'Opening ports in the firewall'],
  ['build.service', 'Setting up the background service'],
  ['launch', 'Starting the server'],
]

/** Steps that may be skipped after they failed (the real list: `kora_core::installer::SKIPPABLE`). */
const SKIPPABLE = new Set(['build.firewall', 'launch'])

/**
 * Stopped installs that can carry on (`POST /installs/{job}/resume`), with their answers. Demo names:
 * "fail..." stops at the database the first time (Try this step again then works); "skip..." stops at
 * starting the server, which may be skipped; "github..." stops because GitHub is not signed in; "nogh..."
 * stops because the GitHub CLI is missing ("Install it for me", then the step is retried by itself);
 * "nodb..." stops because the database server is stopped ("Start it for me", then the same).
 */
const paused = new Map<string, { req: T.InstallRequest; failed: string; attempt: number; done: string[]; skip: Set<string>; noAdmin?: boolean }>()

interface InstallRun {
  /** How many times this install was tried before. */
  attempt: number
  /** Steps finished in an earlier attempt (shown as done again first). */
  done: string[]
  /** Steps the user chose to skip. */
  skip: Set<string>
  /** Windows: "Continue without the administrator parts" was chosen (no prompt any more). */
  noAdmin?: boolean
}

async function runInstall(req: T.InstallRequest, emit: (e: unknown) => void, jobId: () => string, run: InstallRun = { attempt: 0, done: [], skip: new Set() }): Promise<T.InstallOutcome> {
  const lower = req.name.toLowerCase()
  if (run.attempt === 0 && lower.startsWith('nogh')) setMockGhInstalled(false)
  if (run.attempt === 0 && lower.startsWith('nodb')) stopMockDatabase()
  const failAt = lower.startsWith('nogh') && !mockGhInstalled() ? 'prepare.git' : lower.startsWith('nodb') && mockDbStoppedProblem() ? 'build.db' : lower.startsWith('fail') && run.attempt === 0 ? 'build.db' : lower.startsWith('skip') ? 'launch' : lower.startsWith('github') && !prefs.github ? 'prepare.git' : null
  // Windows: one permission prompt before the first step (see ./windowsAdmin).
  if (beh.windows && !run.noAdmin && adminNeeds(req).length > 0) {
    const title = 'Administrator permission'
    emit({ step: ADMIN_STEP, title, status: 'running', detail: 'Windows is asking for permission on the computer that runs Kora. Choose Yes to carry on. If you do not see the prompt, look for a flashing shield icon in the taskbar.', percent: 0 } satisfies T.ProgressEvent)
    await sleep(700)
    const answer = promptAnswer(req.name, run.attempt)
    if (answer !== 'yes') {
      const p = promptProblem(req, answer)
      emit({ step: ADMIN_STEP, title, status: 'failed', detail: p.message, percent: 0 } satisfies T.ProgressEvent)
      const id = jobId()
      paused.set(id, { req, failed: '', attempt: run.attempt + 1, done: run.done, skip: run.skip })
      throw new Fail(403, p.code ?? 'admin.declined', p.message, undefined, { details: p.details, resume: true, kept: [], actions: [...(p.actions ?? []), { kind: 'change_answers' }, { kind: 'open_log', job: id }] })
    }
    emit({ step: ADMIN_STEP, title, status: 'done', detail: 'Permission given: Kora does the administrator steps when the install reaches them', percent: 0 } satisfies T.ProgressEvent)
  }
  const skipped = new Set<string>()
  if (!req.firewall) skipped.add('build.firewall')
  if (!req.service) skipped.add('build.service')
  if (req.db === 'skip') skipped.add('build.db')
  if (!req.start) skipped.add('launch')
  for (let i = 0; i < INSTALL_STEPS.length; i++) {
    const [step, title] = INSTALL_STEPS[i]
    const pct = Math.round(((i + 1) / INSTALL_STEPS.length) * 100)
    if (skipped.has(step) || run.skip.has(step)) {
      emit({ step, title, status: 'skipped', detail: run.skip.has(step) ? 'Skipped: you chose to go on without it' : 'Not needed for your choices', percent: pct } satisfies T.ProgressEvent)
      continue
    }
    if (run.done.includes(step)) {
      emit({ step, title, status: 'done', detail: 'Already done', percent: pct } satisfies T.ProgressEvent)
      continue
    }
    emit({ step, title, status: 'running', detail: 'Working on it', percent: Math.max(0, pct - 10) } satisfies T.ProgressEvent)
    await sleep(step === 'build.fxserver' ? 1800 : 900)
    if (step === failAt) {
      emit({ step, title, status: 'failed', detail: 'This step did not work', percent: pct } satisfies T.ProgressEvent)
      prefs.savedInstall = { mode: compatMode(req), source: req.source, versions: req.versions, name: req.name, dir: req.dir, hostname: req.hostname, ports: req.ports, db: req.db, resources: req.resources, service: req.service, firewall: req.firewall, cfx_identifier: req.cfx_identifier, license_key_name: req.license_key_name ?? null, txadmin_public: req.txadmin_public, artifact_url: req.artifact_url, game_build: req.game_build, github_owner: req.github_owner, start: req.start }
      const id = jobId()
      paused.set(id, { req, failed: step, attempt: run.attempt + 1, done: INSTALL_STEPS.slice(0, i).map(([s]) => s), skip: run.skip })
      const common: Partial<T.JobError> = {
        kept: [`What was already created in ${req.dir}`, 'Your answers (not the licence key or database password)'],
        log_path: `${req.dir}/.kora/install.log`,
        failed_step: step,
        resume: true,
      }
      const tail: T.ProblemAction[] = [
        { kind: 'retry_step', step },
        ...(SKIPPABLE.has(step) ? [{ kind: 'skip_step', step } as T.ProblemAction] : []),
        { kind: 'change_answers' },
        { kind: 'open_log', job: id, path: `${req.dir}/.kora/install.log` },
      ]
      const dbStopped = mockDbStoppedProblem()
      if (step === 'build.db' && dbStopped) {
        throw new Fail(500, dbStopped.code, dbStopped.message, undefined, { ...common, actions: [...(dbStopped.actions ?? []), ...tail] })
      }
      if (step === 'build.db') {
        throw new Fail(500, 'install.db_login_failed', "Could not log in to the database server as 'root'. Kora needs an administrator account to create a NEW database and user. Check the admin user, password, address and port you gave.", undefined, {
          ...common,
          details: "ERROR 1045 (28000): Access denied for user 'root'@'localhost' (using password: YES)",
          actions: [{ kind: 'change_answers', section: 'database' }, ...tail],
        })
      }
      const ghGone = mockGhMissingProblem()
      if (step === 'prepare.git' && ghGone) {
        throw new Fail(500, ghGone.code, ghGone.message, undefined, { ...common, actions: [...(ghGone.actions ?? []), ...tail] })
      }
      if (step === 'prepare.git') {
        throw new Fail(500, 'github.signed_out', 'You are not signed in to GitHub.', undefined, { ...common, actions: [{ kind: 'sign_in_github' }, ...tail] })
      }
      throw new Fail(500, 'install.step_failed', 'The server was installed, but it did not start: txAdmin did not answer within 60 seconds.', undefined, {
        ...common,
        details: '[txAdmin] Error: listen EADDRINUSE: address already in use 0.0.0.0:40120',
        actions: tail,
      })
    }
    emit({ step, title, status: 'done', detail: '', percent: pct } satisfies T.ProgressEvent)
  }
  prefs.savedInstall = null
  servers.push({ name: req.name, dir: req.dir, mode: compatMode(req), source: req.source ?? (req.mode === 'development' ? 'fork' : 'clone'), versions: req.versions ?? (req.mode === 'development' ? 'latest' : 'released'), ports: req.ports, created_unix: Math.floor(Date.now() / 1000), status: req.start ? 'running' : 'stopped', badge: { kind: 'up_to_date' }, managed: true, connect: null })
  installed[req.name] = ['oxmysql', 'ox_lib', 'kora_core']
  return {
    dir: req.dir,
    connect: `${location.hostname}:${req.ports.game}`,
    txadmin_url: `http://${location.hostname}:${req.ports.txadmin}`,
    notes: ['txAdmin asks you to sign in the first time you open it.', ...(req.firewall ? [] : ['The ports are not open in the firewall: players outside this computer cannot connect yet.'])],
    name: req.name,
    start_attempted: req.start,
    running: req.start,
    txadmin_up: req.start,
    game_listening: req.start,
    launch_problem: null,
    log_tail: [],
    admin_user: req.cfx_identifier ? null : 'admin',
    admin_password_file: req.cfx_identifier ? null : `${req.dir}/.kora/admin-password.txt`,
  }
}

// ---- routes ---------------------------------------------------------------------------------

type Body = Record<string, unknown>
type Handler = (m: RegExpMatchArray, body: Body, query: URLSearchParams) => unknown | Promise<unknown>
const routes: [string, RegExp, Handler][] = []
const routeKeys: string[] = []
/** `METHOD /path/{param}` of every route the mock answers (the parity test compares them with the generated route map). */
export const mockRouteKeys = (): readonly string[] => routeKeys
const route = (method: string, pattern: string, h: Handler) => {
  routeKeys.push(`${method} ${pattern}`)
  routes.push([method, new RegExp('^' + pattern.replace(/\{(\w+)\}/g, '([^/]+)') + '$'), h])
}
const dec = (s: string) => decodeURIComponent(s)
/** The headers of the request being answered (the mock routes only get the body). */
let lastHeaders = new Headers()
/** The person chose "Ask for administrator permission" (`X-Kora-Admin: ask`). */
const askedAdmin = () => lastHeaders.get('X-Kora-Admin') === 'ask'
installHubMock(route, startJob, Fail, () => servers)
const keysMock = installKeysMock(route, Fail, () => servers, () => viewer.identity?.role === 'admin')
const reachMock = installReachMock(route, () => servers)
const storeMock = installStoreMock(route, () => [...jobs.values()].map((j) => ({ id: j.id, op: j.op, server: j.server, state: j.state, events: [], started: j.started, ended: j.ended, result: j.result as T.JobSummary['result'], error: j.error })))
const resourceMock = installResourceMock(route, Fail, {
  find: (n) => find(n),
  installed: (n) => installed[n] ?? [],
  consoleEnabled: (n) => !!consoleEnabled[n],
  repos: (n, id) => collabMock.repos(id, find(n)),
  branchRow: (n, id): T.ResourceBranch => (id.startsWith('kora_') ? branchesMock.row(n, id) : { id, folder: id, state: 'pinned', default_branch: 'dev', label: 'fixed version', changed_count: 0, stashes: 0 }),
})
const collabMock = installCollabMock(route, startJob, Fail, {
  find: (n) => find(n),
  installed: (n) => installed[n] ?? [],
  signedIn: () => prefs.github,
  onChanged: (s, id, why) => resourceMock.markChanged(s, [id], why),
  onServerChanged: (s) => pushServer(s),
})
const branchesMock = installBranchesMock(route, startJob, Fail, { find: (n) => find(n), installed: (n) => installed[n] ?? [], signedIn: () => prefs.github, onChanged: (s, id, why) => resourceMock.markChanged(s, [id], why) })

route('GET', '/meta', () => ({ version: prefs.version, api: 1, platform: beh.windows ? 'windows' : 'linux', ops: [], node: 'Mock node (this is demo data)', identity: prefs.signedIn ? viewer.identity : null, remote: viewer.remote, allow_remote_destructive: viewer.identity?.provider === 'tailscale' }) satisfies T.Meta)

// Remote access (demo data: nothing is really opened to the network).
const remoteState: T.RemoteSettings = { enabled: false, enabled_by_flag: false, bind: '127.0.0.1', port: 47120, tailnet_url: null, tailscale: false, tailscale_users: {}, allowed_hosts: [], allow_remote_destructive: false, users: [], saved_to: '/home/demo/.kora/panel.json' }
route('GET', '/remote', () => structuredClone(remoteState))
route('PUT', '/remote', (_m, b) => {
  const u = b as T.RemoteSettingsUpdate
  for (const h of u.allowed_hosts ?? []) if (/[\s/]/.test(h)) throw new Fail(400, 'bad_request', `'${h}' is not a host name or address. Use just the name, for example box.example.ts.net.`)
  Object.assign(remoteState, u)
  return structuredClone(remoteState)
})
route('GET', '/users', () => ({ users: structuredClone(remoteState.users) }))
route('POST', '/users', (_m, b) => {
  const name = String(b.name ?? '').trim()
  if (!/^[A-Za-z0-9._-]{1,40}$/.test(name)) throw new Fail(400, 'bad_request', 'A user name uses letters, numbers, dots, dashes and underscores, up to 40 characters.')
  if (remoteState.users.some((x) => x.name.toLowerCase() === name.toLowerCase())) throw new Fail(409, 'conflict', `There is already a user named '${name}'.`)
  const role = b.role as T.Role
  remoteState.users.push({ name, role, created: Math.floor(Date.now() / 1000) })
  return { name, role, token: 'demo-' + Math.random().toString(16).slice(2).padEnd(56, '0') } satisfies T.NewUser
})
route('DELETE', '/users/{name}', (m) => {
  const n = dec(m[1])
  if (!remoteState.users.some((x) => x.name === n)) throw new Fail(404, 'not_found', `There is no user named '${n}'.`)
  remoteState.users = remoteState.users.filter((x) => x.name !== n)
  return undefined
})
route('POST', '/session/logout', () => undefined)
// One-time codes for sign-in links (the real panel keeps them ten minutes).
const linkCodes = new Set<string>()
route('POST', '/session/link', () => {
  const code = Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, '0')).join('')
  linkCodes.add(code)
  return { code, expires_in: 600 }
})
route('GET', '/system', () => (beh.windows
  ? { admin: false, elevated: false, platform: 'windows', helper_installed: false, headless: false, version: prefs.version, local: !viewer.remote, kora_home: 'C:\\Users\\Sam\\.kora', can_restart: true, admin_prompt: true }
  : { admin: !beh.trouble, elevated: !beh.trouble, platform: 'linux', helper_installed: false, headless: false, version: prefs.version, local: !viewer.remote, kora_home: '/home/sam/.kora', can_restart: true, admin_prompt: false }) satisfies T.SystemInfo)
route('GET', '/servers', () => ({ servers: servers.map((s) => ({ ...s, connect: `192.168.1.20:${s.ports.game}` })) }))
route('POST', '/servers/scan', () =>
  startJob('servers_scan', null, async () => {
    await sleep(1200)
    const have = servers.map((s) => s.dir)
    return { candidates: [{ name: 'weekend-test', dir: '/srv/fivem/weekend-test', mode: 'development', source: 'fork', versions: 'latest', kind: 'txadmin', ports: { game: 30130, txadmin: 40130 } }, { name: 'friends-rp', dir: '/home/sam/friends-rp', mode: 'production', source: 'clone', versions: 'released', kind: 'plain', ports: { game: 30131, txadmin: 40131 } }].filter((c) => !have.includes(c.dir)) }
  }),
)
route('POST', '/servers/adopt', (_m, b) => {
  const dir = String(b.dir ?? '').trim()
  if (!dir.startsWith('/') && !/^[A-Za-z]:\\/.test(dir)) throw new Fail(400, 'bad_request', 'Type the full path of the server folder, for example /srv/fivem/my-server.')
  if (dir.includes('nothing')) throw new Fail(400, 'bad_request', 'That folder does not look like a FiveM server (no server.cfg or resources folder).')
  if (servers.some((s) => s.dir === dir)) throw new Fail(409, 'conflict', 'That server is already in your list.')
  const name = dir.split(/[\\/]/).filter(Boolean).pop() ?? 'adopted'
  const s: T.ServerSummary = { name, dir, mode: 'production', source: 'clone', versions: 'released', ports: { game: nextFree(30123, []), txadmin: nextFree(40123, []) }, created_unix: Math.floor(Date.now() / 1000), status: 'stopped', badge: null, managed: false, connect: null }
  servers.push(s)
  installed[name] = ['chat']
  snapshots[name] = []
  return s
})
route('GET', '/servers/{s}', (m) => detail(find(dec(m[1]))))
route('POST', '/servers/{s}/control', (m, b) => {
  const s = find(dec(m[1]))
  const action = b.action as T.ControlAction
  if (s.status === 'missing') throw new Fail(409, 'conflict', 'The folder for this server is missing, so it cannot be started.')
  return startJob('server_control', s.name, async (emit) => {
    emit({ type: 'status', message: action === 'stop' ? 'Stopping' : action === 'start' ? 'Starting' : 'Restarting' })
    if (action === 'start' && s.status === 'running') return { status: s.status }
    await sleep(1200)
    if ((action === 'start' || action === 'restart') && beh.trouble) throw new Fail(409, 'conflict', `Port ${s.ports.game} is already in use by another program on this computer. Change the port on the Ports tab, or close the other program.`)
    if (action === 'start' || action === 'restart') {
      const clash = servers.find((o) => o.name !== s.name && o.status === 'running' && o.ports.game === s.ports.game)
      if (clash) throw new Fail(409, 'conflict', `Port ${s.ports.game} is already in use by "${clash.name}". Change the port on the Ports tab first.`)
    }
    s.status = action === 'stop' ? 'stopped' : 'running'
    if (action !== 'stop') resourceMock.serverRestarted(s.name)
    pushServer(s)
    return { status: s.status }
  })
})
route('DELETE', '/servers/{s}/registration', (m) => {
  const s = find(dec(m[1]))
  if (s.managed) throw new Fail(400, 'bad_request', 'Kora installed this server, so use Delete server instead.')
  servers.splice(servers.indexOf(s), 1)
  return undefined
})
route('DELETE', '/servers/{s}', (m, b) => {
  const s = find(dec(m[1]))
  if (b.confirm_name !== s.name) throw new Fail(400, 'bad_request', 'The name you typed does not match this server.')
  if (!s.managed) throw new Fail(400, 'bad_request', 'This server was added from an existing folder. You can only remove it from the list.')
  return startJob('server_delete', s.name, async (emit) => {
    for (const t of ['Stopping the server', 'Removing the service', b.keep_files ? 'Keeping your files' : 'Deleting the server folder', b.keep_db ? 'Keeping the database' : 'Deleting the database', 'Removing from the list']) {
      emit({ type: 'status', message: t })
      await sleep(500)
    }
    // Dropping the database needs the admin password when the install did not keep a login (scenario `trouble`).
    if (!b.keep_db && beh.trouble && b.db_admin_password !== 'right-password') {
      throw new Fail(500, 'needs_db_password', b.db_admin_password ? "The database administrator password was not accepted. Check it and try again, or keep the database and delete the server without it." : "Everything else was removed, but the database 'kora_7e870e0e' / user 'kora_7e870e0e' could not be deleted: Kora needs the database administrator password. Enter it and try again, or keep the database and delete the server without it.")
    }
    servers.splice(servers.indexOf(s), 1)
    return { notes: b.keep_files ? [`Your files are still in ${s.dir}`] : [] }
  })
})
route('POST', '/servers/{s}/open-folder', (m) => {
  find(dec(m[1]))
  return undefined
})
route('GET', '/servers/{s}/logs', (m) => {
  find(dec(m[1]))
  return { files: beh.trouble ? [] : LOG_FILES }
})
route('GET', '/servers/{s}/logs/{f}', (m, _b, q) => {
  find(dec(m[1]))
  if (beh.trouble || !LOG_FILES.some((f) => f.name === dec(m[2]))) throw new Fail(404, 'not_found', 'That log file does not exist.')
  return { file: dec(m[2]), lines: fakeLogLines(dec(m[1]), dec(m[2]), Math.min(Number(q.get('lines') ?? 300), 300)) }
})
route('POST', '/servers/{s}/console', (m, b) => {
  const s = find(dec(m[1]))
  if (!consoleEnabled[s.name]) throw new Fail(409, 'conflict', 'The console is not enabled for this server yet.')
  if (s.status !== 'running') throw new Fail(409, 'conflict', 'The server is not running.')
  const cmd = String(b.command ?? '')
  pushConsole(s.name, `> ${cmd}`)
  setTimeout(() => pushConsole(s.name, cmd.startsWith('ensure ') ? `Started resource ${cmd.slice(7)}` : cmd === 'status' ? 'Players online: 3/48' : `Command "${cmd}" ran.`), 250)
  return undefined
})
route('PUT', '/servers/{s}/console/enabled', (m) => {
  const s = find(dec(m[1]))
  consoleEnabled[s.name] = true
  if (!installed[s.name].includes('kora_console')) installed[s.name].push('kora_console')
  return undefined
})
route('GET', '/servers/{s}/resources', (m) => ({ resources: installed[find(dec(m[1])).name] ?? [] }))
route('POST', '/servers/{s}/ensure', (m, b) => {
  const s = find(dec(m[1]))
  const name = String(b.name ?? '')
  if (!/^[A-Za-z0-9_.\-[\]]+$/.test(name)) throw new Fail(400, 'bad_request', 'A resource name can only use letters, numbers, dots, dashes and underscores.')
  if (!installed[s.name].includes(name)) throw new Fail(404, 'not_found', `"${name}" is not installed on this server.`)
  return { message: b.restart ? `Added "ensure ${name}" and restarted ${s.name}.` : `Added "ensure ${name}" to server.cfg. It loads the next time the server restarts.` }
})
route('POST', '/servers/{s}/resources/{name}/control', (m, b) => {
  const s = find(dec(m[1]))
  const name = dec(m[2])
  const action = String(b.action ?? '')
  if (!['start', 'stop', 'restart'].includes(action)) throw new Fail(400, 'bad_request', 'The action must be start, stop or restart.')
  if (action === 'stop' && name === 'kora_console') throw new Fail(400, 'bad_request', 'Kora uses this resource to send commands to the server, so it cannot be stopped from here.')
  if (!installed[s.name].includes(name)) throw new Fail(404, 'not_found', `'${name}' is not installed on this server.`)
  if (s.status !== 'running') throw new Fail(409, 'conflict', 'The server is not running. Start the server first; its resources start with it.')
  if (!consoleEnabled[s.name]) throw new Fail(409, 'conflict', 'The live console is not switched on for this server yet. Turn it on in the Console tab first (it needs a restart).')
  pushConsole(s.name, `> ${action === 'restart' ? 'ensure' : action} ${name}`)
  if (action !== 'stop') resourceMock.resourceRestarted(s.name, name)
  return { message: `Asked the server to ${action} ${name}. The Console tab shows what happened.` }
})
route('POST', '/servers/{s}/update/preview', (m, b) => {
  const s = find(dec(m[1]))
  const only = (b.scope as { resources?: string[] } | undefined)?.resources
  if (only?.length) {
    // One resource: answers quickly and returns just those rows (the other rows are not touched).
    return startJob('update_preview', s.name, async () => {
      await sleep(400)
      const known = (dirtyPreviewed[s.name] ?? makePreview(s)).resources
      const rows = known.filter((r) => only.includes(r.id))
      if (!rows.length) throw new Fail(400, 'bad_request', `This server has no resource named "${only[0]}".`)
      return { new_release: null, resources: rows } satisfies T.UpdatePreview
    })
  }
  return startJob('update_preview', s.name, async () => {
    await sleep(1500)
    const p = makePreview(s)
    // A resource switched to another branch or a pull request is left alone by updates.
    // (The trouble scenario keeps its own examples of resources that need attention.)
    if (!beh.trouble) p.resources = p.resources.map((r) => { const note = branchesMock.offDefault(s.name, r.id); return note ? { ...r, status: 'other_branch', behind: 0, to: r.from, changelog: [], needs_restart: false, note } : r })
    dirtyPreviewed[s.name] = p
    s.badge = badgeFrom(p)
    pushServer(s)
    return p
  })
})
route('POST', '/servers/{s}/update/apply', (m, b) => {
  const s = find(dec(m[1]))
  const skip = (b.skip as string[]) ?? []
  const preview = dirtyPreviewed[s.name] ?? makePreview(s)
  return startJob('update_apply', s.name, async (emit) => {
    const chosen = preview.resources.filter((r) => r.status === 'clean' && !skip.includes(r.id))
    if (!chosen.length) throw new Fail(400, 'bad_request', 'There is nothing selected to update.')
    const snap = `snap-${new Date().toISOString().replace(/[-:T]/g, '').slice(0, 12)}`
    emit({ type: 'snapshot', id: snap })
    await sleep(600)
    for (const [i, r] of chosen.entries()) {
      emit({ type: 'resource', id: r.id, state: 'updating' })
      await sleep(900)
      if (beh.trouble && i === 1) {
        emit({ type: 'resource', id: chosen[0].id, state: 'rolled back' })
        throw new Fail(500, 'failed', `${r.id} could not be updated: its files changed on disk while the update ran. Everything was put back from the safety copy (${snap}).`, undefined, { log_path: `${s.dir}/.kora/update.log` })
      }
      emit({ type: 'resource', id: r.id, state: 'updated' })
    }
    const restart = b.restart as T.RestartPolicy
    if (restart === 'full_restart' && s.status === 'running') emit({ type: 'health', message: 'Restarting and waiting for the server to answer' })
    else emit({ type: 'health', message: 'Checking the files are healthy' })
    await sleep(1000)
    emit({ type: 'health', message: 'Healthy' })
    emit({ type: 'done' })
    ;(snapshots[s.name] ??= []).unshift({ id: snap, created_unix: Math.floor(Date.now() / 1000), resources: chosen.map((c) => c.id), on_other_branch: [] })
    const report: T.UpdateReport = {
      server: s.name,
      updated: chosen.map((c) => ({ id: c.id, from: c.from, to: c.to })),
      skipped: preview.resources.filter((r) => skip.includes(r.id) && r.status === 'clean').map((r) => [r.id, 'You chose to leave it alone']),
      deferred: restart === 'when_quiet' && s.status === 'running' ? [['kora_core', 'Players are online, so the restart is left for later']] : [],
      rolled_back: [],
      sql_flagged: [],
      needs_attention: preview.resources.filter((r) => r.status === 'dirty').map((r) => [r.id, 'It has changes of your own, so it was not touched']),
      forks_synced: s.mode === 'development' ? ['kora_core'] : [],
      forks_left_alone: [],
      not_attempted: [],
      held: [],
      notes: [],
    }
    const left = preview.resources.map((r) => (chosen.includes(r) ? { ...r, status: 'up_to_date' as const, behind: 0, from: r.to } : r))
    dirtyPreviewed[s.name] = { ...preview, resources: left }
    s.badge = badgeFrom(dirtyPreviewed[s.name])
    resourceMock.markChanged(s.name, chosen.map((c) => c.id), 'updated')
    pushServer(s)
    return report
  })
})
route('POST', '/servers/{s}/update/auto', (m) => {
  const s = find(dec(m[1]))
  return startJob('update_auto', s.name, async () => {
    await sleep(1200)
    return {}
  })
})
route('GET', '/servers/{s}/snapshots', (m) => ({ snapshots: snapshots[find(dec(m[1])).name] ?? [] }))
route('POST', '/servers/{s}/snapshots/{id}/rollback', (m) => {
  const s = find(dec(m[1]))
  const snap = (snapshots[s.name] ?? []).find((x) => x.id === dec(m[2]))
  if (!snap) throw new Fail(404, 'not_found', 'That snapshot no longer exists.')
  return startJob('snapshot_rollback', s.name, async (emit) => {
    for (const r of snap.resources) {
      emit({ type: 'resource', id: r, state: 'rolled back' })
      await sleep(600)
    }
    resourceMock.markChanged(s.name, snap.resources, 'rolled back')
    return {}
  })
})
route('GET', '/servers/{s}/catalog', (m) => {
  const s = find(dec(m[1]))
  const cat = { ...catalog, resources: catalog.resources.filter((r) => r.kind !== 'internal' && (s.mode === 'development' || !r.dev_only)) }
  return { catalog: cat, installed: installed[s.name] ?? [], note: s.mode === 'development' ? 'Resources are forked into your GitHub account, so you can change them.' : null } satisfies T.ServerCatalog
})
function planFor(s: T.ServerSummary, ids: string[]): T.AddPlan {
  const have = new Set(installed[s.name])
  const want = new Set<string>()
  const visit = (id: string) => {
    if (want.has(id)) return
    want.add(id)
    catalog.resources.find((r) => r.id === id)?.depends_on.forEach(visit)
  }
  ids.forEach(visit)
  const items: T.PlanItem[] = []
  for (const id of want) {
    const r = catalog.resources.find((x) => x.id === id)
    if (!r || have.has(id)) continue
    const fork = s.mode === 'development' && r.kind === 'kora'
    items.push({ id, rel: `[${r.group}]/${r.id}`, repo: `KoraFiveM/${r.id}`, branch: r.branch, commit: fork ? null : (r.versions[0]?.commit ?? null), fork, third_party: r.group === 'creators' || r.group === 'bridges', subpath: null, dependency: !ids.includes(id), repository: r.repository })
  }
  return { items, already: ids.filter((i) => have.has(i)), ensure: items.map((i) => i.rel.split('/')[0]).filter((v, i, a) => a.indexOf(v) === i) }
}
route('POST', '/servers/{s}/resources/plan', (m, b) => planFor(find(dec(m[1])), (b.ids as string[]) ?? []))
route('POST', '/servers/{s}/resources', (m, b) => {
  const s = find(dec(m[1]))
  const plan = planFor(s, (b.ids as string[]) ?? [])
  if (!plan.items.length) throw new Fail(400, 'bad_request', 'Everything you picked is already installed.')
  return startJob('resources_add', s.name, async (emit) => {
    const out: T.AddOutcome = { installed: [], failed: [], ensured: plan.ensure, notes: [] }
    for (const it of plan.items) {
      emit({ type: 'resource', id: it.id, state: 'installing' })
      await sleep(700)
      if (it.id === 'ns_garage') {
        out.failed.push([it.id, 'The creator\u2019s repository could not be reached.'])
        continue
      }
      installed[s.name].push(it.id)
      out.installed.push(it.id)
      emit({ type: 'resource', id: it.id, state: 'installed' })
    }
    out.notes.push('Restart the server to load the new resources.')
    return out
  })
})
route('GET', '/servers/{s}/webhook', (m) => {
  const u = webhooks[find(dec(m[1])).name]
  return { configured: !!u, set: !!u, redacted: u ? u.replace(/(\/api\/webhooks\/\d+\/).+/, '$1\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022') : null } satisfies T.WebhookState
})
route('PUT', '/servers/{s}/webhook', (m, b) => {
  const s = find(dec(m[1]))
  const url = String(b.url ?? '').trim()
  if (!/^https:\/\/(discord\.com|discordapp\.com)\/api\/webhooks\/\d+\/.+/.test(url)) throw new Fail(400, 'bad_request', 'That does not look like a Discord webhook address. It starts with https://discord.com/api/webhooks/.')
  webhooks[s.name] = url
  return { configured: true, set: true, redacted: url.replace(/(\/api\/webhooks\/\d+\/).+/, '$1\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022') }
})
route('DELETE', '/servers/{s}/webhook', (m) => {
  delete webhooks[find(dec(m[1])).name]
  return undefined
})
route('POST', '/servers/{s}/webhook/test', (m) => {
  if (!webhooks[find(dec(m[1])).name]) throw new Fail(400, 'bad_request', 'Add a webhook first.')
  if (beh.trouble) throw new Fail(502, 'failed', 'Discord did not accept the test message (404 Unknown Webhook). Check that the webhook still exists.')
  return undefined
})
const RUN_AS_HELP = 'As you: updates run with your own GitHub sign-in, so private forks can be fetched, but they only run while your account can run background tasks.\nSystem account: runs even when nobody is signed in, but has no GitHub sign-in, so private forks cannot be fetched.'
const describeSchedule = (c: ScheduleCfg) => `${c.preset === 'daily' ? 'Every day' : `Every ${['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'][c.weekday ?? 0]}`} at ${c.time}, as ${c.run_as === 'system' ? 'the system account' : 'you'}.`
type ScheduleCfg = T.ScheduleConfig
route('GET', '/servers/{s}/schedule', (m) => {
  const s = find(dec(m[1]))
  const c = schedules[s.name] ?? null
  return { schedule: c, description: c ? describeSchedule(c) : null, run_as_help: RUN_AS_HELP } satisfies T.ScheduleState
})
route('PUT', '/servers/{s}/schedule', (m, b) => {
  const s = find(dec(m[1]))
  if (s.mode !== 'development') throw new Fail(400, 'bad_request', 'Scheduled updates are only for development servers.')
  const c = b as unknown as T.ScheduleConfig
  if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(c.time)) throw new Fail(400, 'bad_request', 'The time must be written as HH:MM, for example 04:00.')
  if (c.run_as === 'system' || beh.trouble) throw new Fail(403, 'needs_elevation', 'Changing the automatic updates needs administrator rights. Run the command shown from a terminal opened as administrator, then come back here.', ['kora', 'schedule', 'set', s.dir, c.preset, '--at', c.time, '--run-as', c.run_as])
  schedules[s.name] = c
  return { schedule: c, description: describeSchedule(c), run_as_help: RUN_AS_HELP }
})
route('DELETE', '/servers/{s}/schedule', (m) => {
  if (beh.trouble) throw new Fail(403, 'needs_elevation', 'Changing the automatic updates needs administrator rights. Run the command shown from a terminal opened as administrator, then come back here.', ['kora', 'schedule', 'off', find(dec(m[1])).dir])
  delete schedules[find(dec(m[1])).name]
  return undefined
})
route('PUT', '/servers/{s}/ports', (m, b) => {
  const s = find(dec(m[1]))
  if (s.status === 'running') throw new Fail(409, 'conflict', 'Stop the server before changing its ports.')
  const game = checkPort(b.game)
  const tx = checkPort(b.txadmin)
  if (game === tx) throw new Fail(400, 'bad_request', 'The game port and the txAdmin port must be different.')
  for (const p of [game, tx]) {
    const o = portOwner(p, s.name)
    if (o) throw new Fail(409, 'conflict', `Port ${p} is already used by ${o === 'the database' ? 'the database' : `"${o}"`}. Pick another one.`)
  }
  s.ports = { game, txadmin: tx }
  return { notes: ['server.cfg and the txAdmin settings were updated.', 'If you opened these ports in your firewall or router, open the new ones too.'] }
})
// Paid assets (demo data: nothing is unpacked).
const mockPlan = (zip: string): T.AssetPlan => {
  const name = zip.replace(/\.zip$/i, '').replace(/[^A-Za-z0-9_.-]/g, '') || 'asset'
  const broken = /broken|bomb/i.test(zip)
  return {
    zip_name: zip,
    resources: [{ name, zip_path: `${name}/${name}`, has_fxmanifest: true, has_fxap: !/nofxap/i.test(zip), escrowed: true, size: 18_400_000, files: 214, version: '1.0.4', replaces: Object.values(paidAssets).some((l) => l.some((a) => a.name === name)) }],
    problems: broken
      ? [{ kind: 'zip_bomb', resource: null, message: 'The zip would unpack to far more than its own size; this looks like a zip bomb, so it was refused.', blocking: true }]
      : /nofxap/i.test(zip)
        ? [{ kind: 'missing_fxap', resource: name, message: `${name} looks escrowed but has no hidden .fxap file, so it will not start. Download it again from the Cfx.re Portal and import the zip as it is (do not unpack and re-zip it).`, blocking: false }]
        : [],
    total_size: 18_400_000,
    can_import: !broken,
  }
}
route('GET', '/servers/{s}/assets', (m) => {
  const s = find(dec(m[1]))
  const hints: T.AssetHint[] = beh.trouble && paidAssets[s.name]?.length
    ? [{ kind: 'entitlement', resource: 'qs-housing', line: '[ citizen-server-impl] You lack the required entitlement to use qs-housing', fix: "Your licence key belongs to a different Cfx.re account than the one that bought qs-housing. Open portal.cfx.re → Assets → Granted Assets and transfer it, or use a key from that account." }]
    : []
  return { assets: structuredClone(paidAssets[s.name] ?? []), hints, portal_url: 'https://portal.cfx.re/assets/granted-assets' } satisfies T.PaidAssets
})
route('GET', '/assets/downloads', () => ({ found: true, folder: '/home/demo/Downloads', zips: [{ name: 'jim-mechanic.zip', size: 9_800_000, modified_unix: Math.floor(Date.now() / 1000) - 600 }, { name: 'qs-housing.zip', size: 18_400_000, modified_unix: Math.floor(Date.now() / 1000) - 7200 }] }) satisfies T.RecentDownloads)
route('POST', '/servers/{s}/assets/inspect', (m, b) => {
  find(dec(m[1]))
  if (b.file instanceof Blob) {
    const zip = String(b.fileName ?? 'upload.zip')
    const plan = mockPlan(zip)
    const id = `up${nextId++}`
    if (plan.can_import) uploads.set(id, zip)
    return { plan, upload: plan.can_import ? id : null } satisfies T.AssetInspectResult
  }
  if (typeof b.download === 'string') return { plan: mockPlan(b.download), download: b.download } satisfies T.AssetInspectResult
  throw new Fail(400, 'bad_request', 'Send a zip file, or pick one from your Downloads folder.')
})
route('POST', '/servers/{s}/assets', (m, b) => {
  const s = find(dec(m[1]))
  const zip = typeof b.upload === 'string' ? uploads.get(b.upload) : typeof b.download === 'string' ? b.download : undefined
  if (!zip) throw new Fail(404, 'not_found', 'That upload is gone (uploads are kept for a day). Upload the zip again.')
  const plan = mockPlan(zip)
  return startJob('asset_import', s.name, async (emit) => {
    const out: T.AssetImportReport = { zip_name: zip, imported: [], ensured: [], notes: ['Restart the server to load the new files.'] }
    for (const r of plan.resources) {
      emit({ type: 'status', message: `Unpacking ${r.name}` })
      await sleep(600)
      const list = (paidAssets[s.name] ??= [])
      const old = list.findIndex((a) => a.name === r.name)
      const rec: T.PaidAsset = { name: r.name, path: `[paid]/${r.name}`, version: r.version, imported_unix: Math.floor(Date.now() / 1000), present: true, has_fxap: r.has_fxap, backups: old >= 0 ? list[old].backups + 1 : 0 }
      if (old >= 0) list[old] = rec
      else {
        list.push(rec)
        out.ensured.push(r.name)
        installed[s.name].push(r.name)
      }
      out.imported.push({ name: r.name, version: r.version, replaced: old >= 0, backup: old >= 0 ? `.kora/asset-backups/${r.name}-1` : null })
      emit({ type: 'asset', name: r.name, state: 'imported' })
    }
    return out
  })
})
route('DELETE', '/servers/{s}/assets/{a}', (m) => {
  const s = find(dec(m[1]))
  const name = dec(m[2])
  const list = paidAssets[s.name] ?? []
  if (!list.some((a) => a.name === name)) throw new Fail(404, 'not_found', `There is no paid asset called ${name} on this server.`)
  paidAssets[s.name] = list.filter((a) => a.name !== name)
  installed[s.name] = installed[s.name].filter((r) => r !== name)
  return { notes: [`${name} was moved to .kora/asset-backups (kept as a backup).`, `Removed 'ensure ${name}' from server.cfg.`, 'Restart the server for this to take effect.'] }
})
route('POST', '/servers/{s}/issues', (m, b) => {
  find(dec(m[1]))
  if (!String(b.title ?? '').trim()) throw new Fail(400, 'bad_request', 'Give the issue a short title.')
  if (!prefs.github) throw new Fail(403, 'github.signed_out', 'Sign in to GitHub first.', undefined, { actions: [{ kind: 'sign_in_github' }] })
  return { url: `https://github.com/KoraFiveM/${String(b.resource)}/issues/42` }
})
route('POST', '/servers/{s}/pr/plan', (m, b) => {
  const s = find(dec(m[1]))
  if (s.source !== 'fork') throw new Fail(422, 'collab.needs_fork', 'A pull request is made from your own copy of the resource on GitHub, and this server uses plain copies of the originals. Choose "Make my own copy (fork)" for this resource first.', undefined, { actions: [{ kind: 'make_fork' }] })
  if (!prefs.github) throw new Fail(403, 'github.signed_out', 'Sign in to GitHub first.', undefined, { actions: [{ kind: 'sign_in_github' }] })
  const r = String(b.resource ?? '')
  if (!['kora_core', 'kora_inventory', 'kora_jobs'].includes(r)) throw new Fail(400, 'bad_request', `"${r}" has no changes of your own to send. Make a commit in its folder first.`)
  return { resource: r, upstream: `KoraFiveM/${r}`, base: 'dev', head_owner: 'sam-rp', head_branch: 'fix-payroll', ahead: 2, dir: `${s.dir}/resources/[kora]/${r}` } satisfies T.PrPlan
})
route('POST', '/servers/{s}/pr', (m, b) => {
  find(dec(m[1]))
  if (!String(b.title ?? '').trim()) throw new Fail(400, 'bad_request', 'Give the pull request a title.')
  return { url: 'https://github.com/KoraFiveM/kora_core/pull/87' }
})
const githubStatus = (): T.GithubStatus => {
  // `trouble`: the GitHub CLI is not installed (told apart from "signed out") until the "Install it for me" button has run.
  if (!mockGhInstalled()) {
    return {
      signed_in: false, user: null, state: 'gh_missing', missing_scopes: [],
      problem: { code: 'github.gh_missing', message: 'The GitHub CLI (gh) is not installed. Kora needs it to work with GitHub.', actions: [{ kind: 'install_tool', tool: 'gh' }, { kind: 'open_link', url: 'https://cli.github.com' }] },
    }
  }
  return prefs.github
    ? { signed_in: true, user: 'sam-rp', state: 'ready', missing_scopes: [], problem: null }
    : { signed_in: false, user: null, state: 'signed_out', missing_scopes: [], problem: { code: 'github.signed_out', message: 'You are not signed in to GitHub.', actions: [{ kind: 'sign_in_github' }] } }
}
route('GET', '/github/status', () => githubStatus())
installToolchainMock({ route, startJob, Fail, sleep, github: githubStatus, trouble: () => beh.trouble, windows: () => beh.windows })
route('POST', '/github/login', () => {
  const started = startJob('github_login', null, async () => {
    await sleep(beh.trouble ? 3000 : 6000)
    if (beh.trouble) throw new Fail(400, 'failed', 'GitHub did not confirm the code in time. Start the sign-in again.')
    prefs.github = true
    return { user: 'sam-rp' }
  })
  return { ...started, code: 'WDJB-MJHT', url: 'https://github.com/login/device' } satisfies T.GithubLoginStart
})
route('GET', '/github/owners', () => {
  if (!prefs.github) throw new Fail(403, 'github.signed_out', 'Sign in to GitHub first.', undefined, { actions: [{ kind: 'sign_in_github' }] })
  return { owners: [{ login: 'sam-rp', kind: 'user', can_fork: true, reason: null }, { login: 'sunset-roleplay', kind: 'org', can_fork: false, reason: 'You cannot create repositories in this organisation.' }] satisfies T.GithubOwner[] }
})
route('GET', '/catalog', () => catalog)
route('GET', '/artifacts', () => ({ recommended: 17000, items: [17200, 17100, 17000, 16900, 16800].map((b) => ({ build: b, url: `https://runtime.fivem.net/artifacts/fivem/build_server_windows/master/${b}-abc/server.zip`, date: '2026-02-20' })) }) satisfies T.ArtifactList)
route('GET', '/installs/defaults', () => {
  const taken = servers.map((s) => s.ports.game)
  const game = nextFree(30120, taken)
  return { name: 'my-server', dir: '/srv/fivem/my-server', hostname: 'My Kora server', ports: { game, txadmin: nextFree(40120, servers.map((s) => s.ports.txadmin)) }, saved: prefs.savedInstall && prefs.savedInstall.license_key_name && !keysMock.has(prefs.savedInstall.license_key_name) ? { ...prefs.savedInstall, license_key_name: null } : prefs.savedInstall, profile: keysMock.profile(), resumable: [...paused.entries()].map(([job, p]) => ({ job, name: p.req.name, failed_step: p.failed })), ...(prefs.savedInstall?.license_key_name && !keysMock.has(prefs.savedInstall.license_key_name) ? { saved_key_missing: prefs.savedInstall.license_key_name } : {}) } satisfies T.InstallDefaults
})
route('DELETE', '/installs/saved', () => {
  prefs.savedInstall = null
  return undefined
})
route('POST', '/checks/ports', (_m, b) => {
  const g = checkPort(b.game)
  const t = checkPort(b.txadmin)
  const game = portLine(g, [t])
  const tx = g === t ? { port: t, ok: false, text: 'The game and txAdmin ports must be different.', suggestion: nextFree(t + 1, [g]) } : portLine(t, [g])
  return { game, txadmin: tx } satisfies T.PortsCheck
})
route('POST', '/checks/disk', (_m, b) => {
  const dir = String(b.dir ?? '')
  const low = beh.trouble || dir.includes('small')
  return { ok: !low, free_bytes: low ? 2.1e9 : 148e9, needed_bytes: 6e9, message: low ? 'Only 2.1 GB is free here and about 6 GB is needed. Pick another folder or free some space.' : '148 GB free, about 6 GB needed.' } satisfies T.DiskCheck
})
route('POST', '/checks/db-login', (_m, b) => {
  const a = b.admin as T.DbAdmin | null
  if (beh.trouble || mockDbStoppedProblem() || (a && a.password === 'wrong')) return { ok: false, message: 'The database refused that login. Check the user name and password.', problem: mockDbStoppedProblem() ?? undefined } satisfies T.DbLoginCheck
  return { ok: true, message: a ? 'Logged in to the database.' : 'Kora can log in to the database automatically.' } satisfies T.DbLoginCheck
})
route('POST', '/checks/cfx', (_m, b) => {
  const name = String(b.username ?? '').trim()
  if (!name) throw new Fail(400, 'bad_request', 'Type a Cfx.re username.')
  const none = (message: string) => ({ ok: false, id: null, username: null, identifier: null, message }) satisfies T.CfxCheck
  if (beh.trouble) return none('forum.cfx.re did not answer. Check the internet connection and try again, or skip this step.')
  if (name.toLowerCase().includes('offline')) return none('Could not reach forum.cfx.re. Check the internet connection and try again, or skip this step.')
  if (name.toLowerCase() === 'jsixty') { keysMock.remember({ cfx_username: 'JSixty', cfx_identifier: 'fivem:4465268' }); return { ok: true, id: 4465268, username: 'JSixty', identifier: 'fivem:4465268', message: 'Found JSixty (fivem:4465268). This account will be the txAdmin admin.' } satisfies T.CfxCheck }
  return none(`No Cfx.re account called "${name}" was found. Check the spelling: it is the name on forum.cfx.re.`)
})
/** The parts of an install plan that do not depend on the administrator rights. */
const planBase = (r: T.InstallRequest) => ({
  summary: `${r.mode === 'production' ? 'Production' : 'Development'} server "${r.name}" in ${r.dir}`,
  steps: INSTALL_STEPS.map(([id, title]) => ({ id, title })),
  elevated: false,
  disk: { ok: true, free_bytes: 148e9, needed_bytes: 6e9, message: '148 GB free, about 6 GB needed.' },
  warnings: [] as string[],
  notes: [] as string[],
})
route('POST', '/installs/plan', (_m, b) => {
  const r = b as unknown as T.InstallRequest
  const needs: string[] = []
  if (r.firewall) needs.push('Open the game port in the firewall')
  if (r.txadmin_public) needs.push('Open the txAdmin port in the firewall')
  if (r.service) needs.push('Install the background service')
  if (r.db === 'install_if_missing') needs.push('Install the database if it is missing')
  if (beh.windows) {
    const w = adminNeeds(r)
    return { ...planBase(r), needs_admin: w, admin_prompt: w.length > 0 } satisfies T.InstallPlan
  }
  return { ...planBase(r), needs_admin: needs, admin_prompt: false } satisfies T.InstallPlan
})
route('POST', '/installs', (_m, b) => {
  const r = b as unknown as T.InstallRequest
  if (!r.license_key && !r.license_key_name) throw new Fail(400, 'bad_request', 'A licence key is needed. Get one from the Cfx.re portal.')
  keysMock.forInstall(r)
  if (r.mode === 'development' && r.github_owner) keysMock.remember({ github_owner: r.github_owner })
  if (servers.some((s) => s.name === r.name)) throw new Fail(409, 'conflict', `A server called "${r.name}" already exists.`)
  const clash = portOwner(r.ports.game) ?? portOwner(r.ports.txadmin)
  if (clash) throw new Fail(409, 'conflict', `A port you chose is already in use by ${clash === 'the database' ? 'the database' : `"${clash}"`}. Go back and pick another.`)
  if (beh.windows && r.name.toLowerCase().startsWith('remote') && adminNeeds(r).length > 0 && !askedAdmin()) {
    const p = remoteProblem(r)
    throw new Fail(403, p.code ?? 'admin.remote', p.message, undefined, { actions: p.actions })
  }
  let id = ''
  const started = startJob('install', null, (emit) => runInstall(r, emit, () => id))
  id = started.job
  return started
})
route('POST', '/installs/{j}/resume', (m, b) => {
  const from = dec(m[1])
  const p = paused.get(from)
  if (!p) {
    throw new Fail(404, 'install.cannot_resume', 'This install can no longer be continued from here (Kora Panel was restarted, or it was continued already). Check your answers and start it again: it carries on where it stopped.', undefined, { actions: [{ kind: 'change_answers' }] })
  }
  const skip = typeof b.skip === 'string' ? b.skip : null
  if (skip && (skip !== p.failed || !SKIPPABLE.has(skip))) {
    throw new Fail(409, 'install.cannot_resume', 'This step cannot be skipped: your server needs it. Try it again, or change your answers.', undefined, { actions: [{ kind: 'retry_step', step: p.failed }, { kind: 'change_answers' }] })
  }
  paused.delete(from)
  const noAdmin = b.without_admin === true || !!p.noAdmin
  const run: InstallRun = { attempt: p.attempt, done: p.done, skip: new Set([...p.skip, ...(skip ? [skip] : [])]), noAdmin }
  const req = noAdmin ? withoutAdminReq(p.req) : p.req
  let id = ''
  const started = startJob('install', null, (emit) => runInstall(req, emit, () => id, run))
  id = started.job
  return started
})
route('GET', '/installs/{j}/log', () => ({ lines: ['[install] started', '[install] see the steps above for details'] }))
route('GET', '/self-update', () => ({ current: prefs.version, latest: '0.4.1', available: prefs.version !== '0.4.1', update_available: prefs.version !== '0.4.1', versions: ['0.4.1', '0.4.0', '0.3.4'] }) satisfies T.SelfUpdateInfo)
route('POST', '/self-update', (_m, b) => {
  if (beh.trouble) throw elevate('Updating Kora', ['self-update', '--yes', ...(b.version ? ['--version', String(b.version)] : [])])
  return startJob('self_update', null, async (emit) => {
    emit({ type: 'status', message: 'Downloading' })
    await sleep(1500)
    emit({ type: 'status', message: 'Checking the download' })
    await sleep(800)
    prefs.version = String(b.version ?? '0.4.1')
    return { version: prefs.version }
  })
})
route('POST', '/panel/restart', () => {
  if (beh.trouble) throw new Fail(409, 'conflict', 'This panel cannot restart itself. Run kora panel again.')
  const p = prefs.version.split('.').map(Number)
  p[2] = (p[2] ?? 0) + 1
  prefs.version = p.join('.')
  return { restarting: true }
})
route('GET', '/app', () => ({ installed: prefs.app, version: prefs.app ? prefs.version : null, path: prefs.app ? 'C:\\Program Files\\Kora\\kora.exe' : null, desktop_shortcut: prefs.desktopShortcut, supported: true, legacy: prefs.app || beh.trouble, legacy_items: prefs.app || beh.trouble ? ['Delete C:\\Program Files\\Kora\\kora.exe', 'Remove the shortcut Kora.lnk (it runs as administrator)'] : [], shortcuts: prefs.shortcuts }) satisfies T.AppStatus)
route('PUT', '/app', (_m, b) => {
  prefs.shortcuts = true
  prefs.desktopShortcut = !!b.desktop_shortcut
  if (beh.trouble) throw elevate('Your new Kora shortcuts are ready. Removing the old copy of Kora', ['uninstall', '--app', '--yes'])
  prefs.app = false
  return { message: 'Done. Kora now opens from your own shortcuts. Your servers were not touched.' }
})
route('DELETE', '/app', () => {
  if (beh.trouble) throw elevate('Removing the old Kora app', ['uninstall', '--app', '--yes'])
  prefs.app = false
  return { message: 'The old Kora app was removed.' }
})
route('POST', '/uninstall/plan', (_m, b) => {
  const only = b.server ? find(String(b.server)) : null
  const items: T.UninstallItem[] = []
  for (const s of only ? [only] : servers.filter((x) => x.managed)) {
    items.push(
      { item: `${s.name}/service`, description: `Delete Kora's service definition /etc/systemd/system/kora-${s.name}.service`, kind: 'system', remove: true },
      { item: `${s.name}/firewall`, description: `Close the firewall ports ${s.ports.game} and ${s.ports.txadmin} that Kora opened`, kind: 'system', remove: true },
      { item: `${s.name}/registry`, description: `Forget '${s.name}' in the Kora server list (its folder and database stay)`, kind: 'system', remove: true },
      { item: `${s.name}/folder`, description: `Delete the server folder ${s.dir} and what Kora put in it`, kind: 'folder', remove: false },
      { item: `${s.name}/database`, description: `Drop the database and database user Kora created for '${s.name}'`, kind: 'database', remove: false },
    )
  }
  if (!only && prefs.app) items.push({ item: 'app', description: "Kora's shortcuts (and the old machine-wide Kora app, if it is still installed). Your servers are not touched", kind: 'app', remove: false })
  return { items } satisfies T.UninstallPlan
})
route('POST', '/uninstall', (_m, b) => {
  const only = (b.only as string[] | undefined) ?? []
  if (beh.trouble) throw elevate('Removing services, firewall rules and accounts', ['uninstall', '--yes', ...(b.delete_files ? ['--delete-files'] : []), ...(b.delete_databases ? ['--delete-databases'] : []), ...(only.length === 1 ? only : [])])
  const answers = (b.answers as { item: string; remove: boolean }[] | undefined) ?? []
  return startJob('uninstall', only.length === 1 ? only[0] : null, async (emit) => {
    const removed: string[] = []
    for (const a of answers.filter((x) => x.remove)) {
      emit({ type: 'status', message: `Removing ${a.item}` })
      removed.push(a.item)
      await sleep(500)
    }
    const kept = answers.filter((x) => !x.remove).map((x) => x.item)
    return { removed, kept, notes: ['Your server folders and databases were kept unless you chose to delete them.', ...kept.map((k) => `Kept ${k}`)] }
  })
})
route('GET', '/jobs/{id}', (m) => {
  const j = jobs.get(dec(m[1]))
  if (!j) throw new Fail(404, 'not_found', 'That job is not known (it may be from before Kora restarted).')
  return jobView(j)
})

installMonitorMock({ route, servers, fail: (status, code, message) => new Fail(status, code, message) })

// ---- fetch shim -----------------------------------------------------------------------------

const json = (status: number, body: unknown) => new Response(body === undefined ? null : JSON.stringify(body), { status: body === undefined && status === 200 ? 204 : status, headers: { 'Content-Type': 'application/json' } })
const errRes = (e: Fail) =>
  json(e.status, { error: { code: e.code, message: e.message, ...(e.command ? { command: e.command } : {}), ...(e.extra.details ? { details: e.extra.details } : {}), ...(e.extra.actions ? { actions: e.extra.actions } : {}) } })

async function handle(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.origin)
  const method = (init?.method ?? 'GET').toUpperCase()
  const path = url.pathname.slice(BASE.length)
  const [lo, hi] = beh.latency
  const wait = lo + (hi - lo) * rand()
  await new Promise<void>((r) => setTimeout(r, wait * (speed === 0 ? 0 : 1)))
  if (beh.auth === 'down') throw new TypeError('Failed to fetch')
  try {
    if (beh.auth === 'expired') throw new Fail(401, 'unauthorized', 'You are not signed in.')
    if (path === '/session' && method === 'POST') {
      // Like the real panel: a pasted token, or the one-time code from a sign-in link (`/#code=...`).
      const b = JSON.parse(String(init?.body ?? '{}')) as { token?: string; code?: string }
      const ok = b.code ? linkCodes.delete(b.code) : !!b.token && b.token !== 'bad'
      if (!ok) {
        prefs.signedIn = false
        throw new Fail(401, 'unauthorized', 'That sign-in link is not valid or has expired.')
      }
      prefs.signedIn = true
      return json(200, { ok: true, identity: { name: 'local', provider: 'local', role: 'admin' }, secret: 'mock-session-secret' } satisfies T.SessionCreated)
    }
    if (!prefs.signedIn) throw new Fail(401, 'unauthorized', 'You are not signed in.')
    const headers = new Headers(init?.headers)
    lastHeaders = headers
    if (method !== 'GET' && !headers.has('X-Kora')) throw new Fail(403, 'forbidden', 'Missing the X-Kora header.')
    const raw = init?.body
    const body: Body = raw instanceof Blob ? { file: raw, fileName: decodeURIComponent(headers.get('X-File-Name') ?? 'upload.zip') } : raw ? (JSON.parse(String(raw)) as Body) : {}
    for (const [mth, re, h] of routes) {
      const m = path.match(re)
      if (mth === method && m) {
        const out = await h(m, body, url.searchParams)
        if (out && typeof out === 'object' && 'job' in out) return json(202, out)
        return json(out === undefined ? 204 : 200, out)
      }
    }
    throw new Fail(404, 'not_found', 'That page of the Kora API does not exist.')
  } catch (e) {
    if (e instanceof Fail) return errRes(e)
    return errRes(new Fail(500, 'internal', e instanceof Error ? e.message : 'Something went wrong.'))
  }
}

// ---- EventSource shim -----------------------------------------------------------------------

class MockEventSource {
  readonly url: string
  readyState = 0
  onerror: ((e: Event) => void) | null = null
  onmessage: ((e: MessageEvent) => void) | null = null
  onopen: ((e: Event) => void) | null = null
  private listeners = new Map<string, Set<(e: MessageEvent) => void>>()
  private cleanup: (() => void) | null = null
  constructor(url: string) {
    this.url = url
    setTimeout(() => this.connect(), 30)
  }
  addEventListener(type: string, fn: (e: MessageEvent) => void) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set())
    this.listeners.get(type)!.add(fn)
  }
  removeEventListener(type: string, fn: (e: MessageEvent) => void) {
    this.listeners.get(type)?.delete(fn)
  }
  close() {
    this.readyState = 2
    this.cleanup?.()
  }
  private emit: Fn = (kind, data) => {
    if (this.readyState === 2) return
    const ev = new MessageEvent(kind, { data: JSON.stringify(data) })
    this.listeners.get(kind)?.forEach((f) => f(ev))
    if (kind === 'message') this.onmessage?.(ev)
  }
  private connect() {
    if (this.readyState === 2) return
    if (beh.auth !== 'ok') return void this.onerror?.(new Event('error'))
    this.readyState = 1
    this.onopen?.(new Event('open'))
    const path = new URL(this.url, location.origin).pathname.slice(BASE.length)
    let m: RegExpMatchArray | null
    if ((m = path.match(/^\/jobs\/([^/]+)\/events$/))) {
      const j = jobs.get(dec(m[1]))
      if (!j) return void this.onerror?.(new Event('error'))
      j.events.forEach((e) => this.emit('progress', e))
      if (j.state !== 'running') return void this.emit(j.state === 'done' ? 'done' : 'failed', {})
      j.subs.add(this.emit)
      this.cleanup = () => j.subs.delete(this.emit)
    } else if (path === '/events') {
      serverBus.add(this.emit)
      this.cleanup = () => serverBus.delete(this.emit)
    } else if ((m = path.match(/^\/servers\/([^/]+)\/console\/stream$/))) {
      const name = dec(m[1])
      if (!consoleEnabled[name]) return void this.onerror?.(new Event('error'))
      if (!consoleBus.has(name)) consoleBus.set(name, new Set())
      consoleBus.get(name)!.add(this.emit)
      this.emit('line', { line: `Connected to the ${name} console.` })
      this.cleanup = () => consoleBus.get(name)?.delete(this.emit)
    } else this.onerror?.(new Event('error'))
  }
}

export interface MockOptions {
  scenario?: ScenarioName
  /** Time multiplier for waits (1 = normal, 0 = no waiting; used by tests). */
  speed?: number
}

export function activeScenario(): ScenarioName {
  return scenario
}

export function installMock(opts: MockOptions = {}): ScenarioName {
  const name = opts.scenario ?? resolveScenario(window.location.search, safeStorage())
  init(name)
  viewer = resolveViewer(window.location.search, safeStorage())
  if (opts.speed !== undefined) speed = opts.speed
  startTicker()
  const real = realFetch ?? (realFetch = window.fetch.bind(window))
  window.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    const u = typeof input === 'string' ? input : input instanceof URL ? input.pathname : input.url
    return u.startsWith(BASE) ? handle(input, init) : real(input, init)
  }) as typeof window.fetch
  window.EventSource = MockEventSource as unknown as typeof EventSource
  console.info(`[kora] Using the built-in mock backend (scenario "${name}"). All data is fake.`)
  return name
}

let realFetch: typeof window.fetch | null = null

/**
 * Who is looking, for the browser tests of what other people see: `?as=operator` (this computer, an operator),
 * `?as=remote-operator` (another device through Tailscale), `?as=remote-admin` (another device with an admin token), `?as=remote-tailscale-admin` (another device, a Tailscale administrator).
 * Remembered for the tab like the scenario; `?as=local` goes back to the default.
 */
type Viewer = { identity: T.Meta['identity']; remote: boolean }
const VIEWERS: Record<string, Viewer> = {
  operator: { identity: { name: 'kim', provider: 'local', role: 'operator' }, remote: false },
  'remote-operator': { identity: { name: 'kim', provider: 'tailscale', role: 'operator' }, remote: true },
  'remote-admin': { identity: { name: 'sam', provider: 'token', role: 'admin' }, remote: true },
  'remote-tailscale-admin': { identity: { name: 'ada', provider: 'tailscale', role: 'admin' }, remote: true },
}
let viewer: Viewer = { identity: { name: 'local', provider: 'local', role: 'admin' }, remote: false }
function resolveViewer(search: string, storage: Storage | null): Viewer {
  const key = 'kora-mock-as'
  try {
    const q = new URLSearchParams(search).get('as')
    if (q) {
      if (VIEWERS[q]) storage?.setItem(key, q)
      else storage?.removeItem(key)
    }
    const saved = storage?.getItem(key)
    if (saved && VIEWERS[saved]) return VIEWERS[saved]
  } catch {
    /* storage blocked: the default viewer */
  }
  return { identity: { name: 'local', provider: 'local', role: 'admin' }, remote: false }
}
function safeStorage(): Storage | null {
  try {
    return window.sessionStorage
  } catch {
    return null
  }
}
