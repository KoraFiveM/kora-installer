// In-app mock backend: replaces window.fetch (for /api/v1) and window.EventSource.
// Only loaded when VITE_MOCK=1 (or `npm run dev`), see src/main.tsx.
import type * as T from '../types'
import { CONSOLE_LINES, INSTALLED, LOG_FILES, fakeLogLines, seedCatalog, seedServers, seedSnapshots } from './data'
import { behaviourOf, resolveScenario, seeded, type Behaviour, type ScenarioName } from './scenarios'

const BASE = '/api/v1'

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
let prefs = { github: false as boolean, savedInstall: null as T.SavedInstall | null, app: false, desktopShortcut: false, version: '0.4.0', signedIn: true }
let dirtyPreviewed: Record<string, T.UpdatePreview> = {}
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
  prefs = { github: false, savedInstall: null, app: false, desktopShortcut: false, version: '0.4.0', signedIn: true }
  dirtyPreviewed = {}
  jobs.clear()
  busy.clear()
  nextId = 1
  consoleTick = 0
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
const elevate = (what: string, cmd: string[]) => new Fail(403, 'needs_elevation', `${what} needs administrator rights. Run this command as administrator:`, ['kora', ...cmd])

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
  subs: Set<Fn>
}
const jobs = new Map<string, MJob>()
const busy = new Set<string>()

function startJob(op: string, server: string | null, run: (emit: (e: unknown) => void) => Promise<unknown>, opts: { exclusive?: boolean } = {}): T.JobStarted {
  if (server && opts.exclusive !== false) {
    if (busy.has(server)) throw new Fail(409, 'conflict', `Another job is already running on "${server}". Wait for it to finish and try again.`)
    busy.add(server)
  }
  const job: MJob = { id: `job-${nextId++}`, op, server, state: 'running', events: [], result: null, error: null, subs: new Set() }
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
    } catch (e) {
      job.state = 'failed'
      job.error = e instanceof Fail ? { code: e.code, message: e.message, command: e.command, ...e.extra } : { code: 'internal', message: e instanceof Error ? e.message : 'Something went wrong.' }
    }
    if (server) busy.delete(server)
    job.subs.forEach((f) => f(job.state === 'done' ? 'done' : 'failed', {}))
  })()
  return { job: job.id }
}

const jobView = (j: MJob): T.Job => ({ id: j.id, op: j.op, server: j.server, state: j.state, events: j.events, result: j.result, error: j.error })

// ---- helpers --------------------------------------------------------------------------------

const detail = (s: T.ServerSummary): T.ServerDetail => ({
  ...s,
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

async function runInstall(req: T.InstallRequest, emit: (e: unknown) => void): Promise<T.InstallOutcome> {
  const fail = req.name.toLowerCase().startsWith('fail')
  const skipped = new Set<string>()
  if (!req.firewall) skipped.add('build.firewall')
  if (!req.service) skipped.add('build.service')
  if (req.db === 'skip') skipped.add('build.db')
  if (!req.start) skipped.add('launch')
  for (let i = 0; i < INSTALL_STEPS.length; i++) {
    const [step, title] = INSTALL_STEPS[i]
    const pct = Math.round(((i + 1) / INSTALL_STEPS.length) * 100)
    if (skipped.has(step)) {
      emit({ step, title, status: 'skipped', detail: 'Not needed for your choices', percent: pct } satisfies T.ProgressEvent)
      continue
    }
    emit({ step, title, status: 'running', detail: 'Working on it', percent: Math.max(0, pct - 10) } satisfies T.ProgressEvent)
    await sleep(step === 'build.fxserver' ? 1800 : 900)
    if (fail && step === 'build.db') {
      emit({ step, title, status: 'failed', detail: 'Could not log in to the database', percent: pct } satisfies T.ProgressEvent)
      prefs.savedInstall = { mode: req.mode, name: req.name, dir: req.dir, hostname: req.hostname, ports: req.ports, db: req.db, resources: req.resources, service: req.service, firewall: req.firewall }
      throw new Fail(500, 'internal', 'The database refused the administrator login, so the install stopped before creating the server database.', undefined, {
        kept: [`The FiveM server files in ${req.dir}`, 'Your answers (use "Try again" to continue)'],
        log_path: `${req.dir}/.kora/install.log`,
      })
    }
    emit({ step, title, status: 'done', detail: '', percent: pct } satisfies T.ProgressEvent)
  }
  prefs.savedInstall = null
  servers.push({ name: req.name, dir: req.dir, mode: req.mode, ports: req.ports, created_unix: Math.floor(Date.now() / 1000), status: req.start ? 'running' : 'stopped', badge: { kind: 'up_to_date' }, managed: true })
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
    admin_user: 'admin',
    admin_password_file: `${req.dir}/.kora/admin-password.txt`,
  }
}

// ---- routes ---------------------------------------------------------------------------------

type Body = Record<string, unknown>
type Handler = (m: RegExpMatchArray, body: Body, query: URLSearchParams) => unknown | Promise<unknown>
const routes: [string, RegExp, Handler][] = []
const route = (method: string, pattern: string, h: Handler) => {
  routes.push([method, new RegExp('^' + pattern.replace(/\{(\w+)\}/g, '([^/]+)') + '$'), h])
}
const dec = (s: string) => decodeURIComponent(s)

route('GET', '/meta', () => ({ version: prefs.version, api: 1, platform: 'linux', ops: [], node: 'Mock node (this is demo data)' }) satisfies T.Meta)
route('GET', '/system', () => ({ admin: !beh.trouble, elevated: !beh.trouble, platform: 'linux', helper_installed: false, headless: false, version: prefs.version, local: true, kora_home: '/home/sam/.kora' }) satisfies T.SystemInfo)
route('GET', '/servers', () => ({ servers: servers.map((s) => ({ ...s })) }))
route('POST', '/servers/scan', () =>
  startJob('servers_scan', null, async () => {
    await sleep(1200)
    const have = servers.map((s) => s.dir)
    return { candidates: [{ name: 'weekend-test', dir: '/srv/fivem/weekend-test', mode: 'development', kind: 'txadmin', ports: { game: 30130, txadmin: 40130 } }, { name: 'friends-rp', dir: '/home/sam/friends-rp', mode: 'production', kind: 'plain', ports: { game: 30131, txadmin: 40131 } }].filter((c) => !have.includes(c.dir)) }
  }),
)
route('POST', '/servers/adopt', (_m, b) => {
  const dir = String(b.dir ?? '').trim()
  if (!dir.startsWith('/') && !/^[A-Za-z]:\\/.test(dir)) throw new Fail(400, 'bad_request', 'Type the full path of the server folder, for example /srv/fivem/my-server.')
  if (dir.includes('nothing')) throw new Fail(400, 'bad_request', 'That folder does not look like a FiveM server (no server.cfg or resources folder).')
  if (servers.some((s) => s.dir === dir)) throw new Fail(409, 'conflict', 'That server is already in your list.')
  const name = dir.split(/[\\/]/).filter(Boolean).pop() ?? 'adopted'
  const s: T.ServerSummary = { name, dir, mode: 'production', ports: { game: nextFree(30123, []), txadmin: nextFree(40123, []) }, created_unix: Math.floor(Date.now() / 1000), status: 'stopped', badge: null, managed: false }
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
route('POST', '/servers/{s}/update/preview', (m) => {
  const s = find(dec(m[1]))
  return startJob('update_preview', s.name, async () => {
    await sleep(1500)
    const p = makePreview(s)
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
    ;(snapshots[s.name] ??= []).unshift({ id: snap, created_unix: Math.floor(Date.now() / 1000), resources: chosen.map((c) => c.id) })
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
    }
    const left = preview.resources.map((r) => (chosen.includes(r) ? { ...r, status: 'up_to_date' as const, behind: 0, from: r.to } : r))
    dirtyPreviewed[s.name] = { ...preview, resources: left }
    s.badge = badgeFrom(dirtyPreviewed[s.name])
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
route('POST', '/servers/{s}/issues', (m, b) => {
  find(dec(m[1]))
  if (!String(b.title ?? '').trim()) throw new Fail(400, 'bad_request', 'Give the issue a short title.')
  if (!prefs.github) throw new Fail(403, 'forbidden', 'Sign in to GitHub first.')
  return { url: `https://github.com/KoraFiveM/${String(b.resource)}/issues/42` }
})
route('POST', '/servers/{s}/pr/plan', (m, b) => {
  const s = find(dec(m[1]))
  if (s.mode !== 'development') throw new Fail(400, 'bad_request', 'Pull requests are only for development servers.')
  if (!prefs.github) throw new Fail(403, 'forbidden', 'Sign in to GitHub first.')
  const r = String(b.resource ?? '')
  if (r !== 'kora_core' && r !== 'kora_inventory') throw new Fail(400, 'bad_request', `"${r}" has no changes of your own to send. Make a commit in its folder first.`)
  return { resource: r, upstream: `KoraFiveM/${r}`, base: 'dev', head_owner: 'sam-rp', head_branch: 'fix-payroll', ahead: 2, dir: `${s.dir}/resources/[kora]/${r}` } satisfies T.PrPlan
})
route('POST', '/servers/{s}/pr', (m, b) => {
  find(dec(m[1]))
  if (!String(b.title ?? '').trim()) throw new Fail(400, 'bad_request', 'Give the pull request a title.')
  return { url: 'https://github.com/KoraFiveM/kora_core/pull/87' }
})
route('GET', '/github/status', () => ({ signed_in: prefs.github, user: prefs.github ? 'sam-rp' : null }) satisfies T.GithubStatus)
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
  if (!prefs.github) throw new Fail(403, 'forbidden', 'Sign in to GitHub first.')
  return { owners: [{ login: 'sam-rp', kind: 'user', can_fork: true, reason: null }, { login: 'sunset-roleplay', kind: 'org', can_fork: false, reason: 'You cannot create repositories in this organisation.' }] satisfies T.GithubOwner[] }
})
route('GET', '/catalog', () => catalog)
route('GET', '/artifacts', () => ({ recommended: 17000, items: [17200, 17100, 17000, 16900, 16800].map((b) => ({ build: b, url: `https://runtime.fivem.net/artifacts/fivem/build_server_windows/master/${b}-abc/server.zip`, date: '2026-02-20' })) }) satisfies T.ArtifactList)
route('GET', '/installs/defaults', () => {
  const taken = servers.map((s) => s.ports.game)
  const game = nextFree(30120, taken)
  return { name: 'my-server', dir: '/srv/fivem/my-server', hostname: 'My Kora server', ports: { game, txadmin: nextFree(40120, servers.map((s) => s.ports.txadmin)) }, saved: prefs.savedInstall } satisfies T.InstallDefaults
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
  if (beh.trouble || (a && a.password === 'wrong')) return { ok: false, message: 'The database refused that login. Check the user name and password.' }
  return { ok: true, message: a ? 'Logged in to the database.' : 'Kora can log in to the database automatically.' } satisfies T.DbLoginCheck
})
route('POST', '/installs/plan', (_m, b) => {
  const r = b as unknown as T.InstallRequest
  const needs: string[] = []
  if (r.firewall) needs.push('Open the game port in the firewall')
  if (r.txadmin_public) needs.push('Open the txAdmin port in the firewall')
  if (r.service) needs.push('Install the background service')
  if (r.db === 'install_if_missing') needs.push('Install the database if it is missing')
  return { steps: INSTALL_STEPS.map(([id, title]) => ({ id, title })), needs_admin: needs, notes: [] } satisfies T.InstallPlan
})
route('POST', '/installs', (_m, b) => {
  const r = b as unknown as T.InstallRequest
  if (!r.license_key) throw new Fail(400, 'bad_request', 'A licence key is needed. Get one from the Cfx.re portal.')
  if (servers.some((s) => s.name === r.name)) throw new Fail(409, 'conflict', `A server called "${r.name}" already exists.`)
  const clash = portOwner(r.ports.game) ?? portOwner(r.ports.txadmin)
  if (clash) throw new Fail(409, 'conflict', `A port you chose is already in use by ${clash === 'the database' ? 'the database' : `"${clash}"`}. Go back and pick another.`)
  return startJob('install', null, (emit) => runInstall(r, emit))
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
route('GET', '/app', () => ({ installed: prefs.app, version: prefs.app ? prefs.version : null, path: prefs.app ? 'C:\\Users\\sam\\AppData\\Local\\Kora\\kora.exe' : null, desktop_shortcut: prefs.desktopShortcut, supported: true }) satisfies T.AppStatus)
route('PUT', '/app', (_m, b) => {
  if (beh.trouble) throw elevate('Installing the Kora app', ['app', 'install', '--yes', ...(b.desktop_shortcut ? [] : ['--no-desktop-shortcut'])])
  prefs.app = true
  prefs.desktopShortcut = !!b.desktop_shortcut
  return { message: 'The Kora app is installed.' }
})
route('DELETE', '/app', () => {
  if (beh.trouble) throw elevate('Removing the Kora app', ['uninstall', '--app', '--yes'])
  prefs.app = false
  return { message: 'The Kora app was removed.' }
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
  if (!only && prefs.app) items.push({ item: 'app', description: 'The Kora app (the `kora` command, shortcuts and the installed program)', kind: 'app', remove: false })
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

// ---- fetch shim -----------------------------------------------------------------------------

const json = (status: number, body: unknown) => new Response(body === undefined ? null : JSON.stringify(body), { status: body === undefined && status === 200 ? 204 : status, headers: { 'Content-Type': 'application/json' } })
const errRes = (e: Fail) => json(e.status, { error: { code: e.code, message: e.message, ...(e.command ? { command: e.command } : {}) } })

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
      const token = (JSON.parse(String(init?.body ?? '{}')) as { token?: string }).token
      if (!token || token === 'bad') {
        prefs.signedIn = false
        throw new Fail(401, 'unauthorized', 'That sign-in link is not valid or has expired.')
      }
      prefs.signedIn = true
      return json(204, undefined)
    }
    if (!prefs.signedIn) throw new Fail(401, 'unauthorized', 'You are not signed in.')
    const headers = new Headers(init?.headers)
    if (method !== 'GET' && !headers.has('X-Kora')) throw new Fail(403, 'forbidden', 'Missing the X-Kora header.')
    const body = init?.body ? (JSON.parse(String(init.body)) as Body) : {}
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
function safeStorage(): Storage | null {
  try {
    return window.sessionStorage
  } catch {
    return null
  }
}
