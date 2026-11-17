// Mock scenarios: which fake world the in-app backend simulates. Chosen with `?scenario=<name>` (kept in
// sessionStorage so navigating keeps it) or with the floating "Mock" switcher. Only bundled in mock builds.

export const SCENARIOS = ['default', 'empty', 'fleet', 'trouble', 'slow', 'offline', 'down', 'windows', 'service', 'noservice', 'notallowed', 'restarted', 'keylost'] as const
export type ScenarioName = (typeof SCENARIOS)[number]

export const SCENARIO_INFO: Record<ScenarioName, string> = {
  default: '3 servers (running, stopped with updates, folder missing). Everything works.',
  empty: 'No servers and no saved install answers: the first-run screens.',
  fleet: '31 servers: every mode x status x update badge, 32-character names, high ports, one added from an existing folder.',
  trouble: 'Everything fails in realistic ways: port conflict, dirty updates, failed apply, administrator rights, webhook, console, logs, GitHub, disk, database.',
  slow: 'Like default, but every request takes 1.5 to 3 s and jobs run slowly (loading states).',
  offline: 'The API answers 401: the "session expired" screen.',
  down: 'Every request fails with a network error (Kora is not answering).',
  service: "Not an administrator, with Kora's administrator service installed and turned on for you: the administrator parts of an install just happen (no prompt).",
  noservice: "A per-user install without Kora's administrator service: the administrator parts offer the choices (install the service, continue without them, the command).",
  notallowed: "Kora's administrator service is installed but not turned on for your account: the administrator parts say so and show the command an administrator runs (kora service allow).",
  windows: "Windows, a per-user install without Kora's administrator service: the administrator parts offer 'Install Kora's administrator service', which runs the installer (one permission prompt) and then works.",
  restarted: 'The panel was restarted: Settings shows the jobs and activity from before, one install was stopped by the restart (Carry on), one update job was interrupted.',
  keylost: 'Like default, with one saved licence key (Main) whose system keychain entry has disappeared: Settings says so and offers to enter it again.',
}

export const STORAGE_KEY = 'kora-mock-scenario'

type Store = Pick<Storage, 'getItem' | 'setItem'>

export function isScenario(v: unknown): v is ScenarioName {
  return typeof v === 'string' && (SCENARIOS as readonly string[]).includes(v)
}

/** `?scenario=` wins (and is remembered); otherwise the remembered one; otherwise `default`. */
export function resolveScenario(search: string, storage: Store | null): ScenarioName {
  const fromQuery = new URLSearchParams(search).get('scenario')
  try {
    if (isScenario(fromQuery)) {
      storage?.setItem(STORAGE_KEY, fromQuery)
      return fromQuery
    }
    const saved = storage?.getItem(STORAGE_KEY)
    if (isScenario(saved)) return saved
  } catch {
    /* storage blocked: fall through */
  }
  return 'default'
}

export interface Behaviour {
  /** Delay of every request, in ms: [min, max] (picked with a seeded generator, so it is repeatable). */
  latency: [number, number]
  /** Multiplier for the time jobs take. */
  jobSlow: number
  auth: 'ok' | 'expired' | 'down'
  /** Everything fails in realistic ways. */
  trouble: boolean
  /** The Windows panel (paths, wording). The windows scenario is also a per-user install without the service. */
  windows: boolean
  /** Not an administrator: Kora's administrator service is `ready` or `missing` (`admin_service`). `none` = the scenario does not say. */
  service: 'none' | 'ready' | 'missing' | 'not_allowed'
}

export function behaviourOf(n: ScenarioName): Behaviour {
  const base: Behaviour = { latency: [120, 120], jobSlow: 1, auth: 'ok', trouble: false, windows: false, service: 'none' }
  switch (n) {
    case 'slow':
      return { ...base, latency: [1500, 3000], jobSlow: 3 }
    case 'offline':
      return { ...base, auth: 'expired' }
    case 'down':
      return { ...base, auth: 'down' }
    case 'trouble':
      return { ...base, trouble: true }
    case 'windows':
      return { ...base, windows: true, service: 'missing' }
    case 'service':
      return { ...base, service: 'ready' }
    case 'notallowed':
      return { ...base, service: 'not_allowed' }
    case 'noservice':
      return { ...base, service: 'missing' }
    default:
      return base
  }
}

/** Small seeded generator (mulberry32): the same seed always gives the same numbers. */
export function seeded(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/** Point the page at another scenario (or reload the current one to reset its state). */
export function switchScenario(n: ScenarioName): void {
  try {
    sessionStorage.setItem(STORAGE_KEY, n)
  } catch {
    /* ignore */
  }
  const u = new URL(window.location.href)
  u.searchParams.set('scenario', n)
  window.location.assign(u.toString())
}
