// Mock data and routes for the job history (`GET /jobs`) and the audit log (`GET /audit`), like the real panel's:
// the history survives a "restart" (it is seeded with jobs from before), and one install was stopped by a restart.
// `installStoreMock` is called once from mock/index.ts.
import type { AuditRow, JobSummary } from '../store'

type Body = Record<string, unknown>
type Route = (method: string, pattern: string, h: (m: RegExpMatchArray, body: Body, query: URLSearchParams) => unknown | Promise<unknown>) => void

const NOW = Math.floor(Date.now() / 1000)
const ago = (mins: number) => NOW - mins * 60

export const seedJobs = (): JobSummary[] => [
  { id: 'old-3', op: 'install_start', server: 'Staging', state: 'interrupted', started: ago(95), ended: ago(60), result: null, error: { code: 'job.interrupted', message: 'Kora Panel was closed, or the computer restarted, while this install for \'Staging\' was running, so it did not finish. Kora kept your answers: open Install and choose Carry on, and it continues from the last step that finished.', resume: true } },
  { id: 'old-2', op: 'update_apply', server: 'Main roleplay', state: 'failed', started: ago(300), ended: ago(298), result: null, error: { code: 'failed', message: 'The update could not be downloaded. Check the internet connection and try again.' } },
  { id: 'old-1', op: 'update_apply', server: 'Main roleplay', state: 'done', started: ago(1500), ended: ago(1498), result: { updated: 3 }, error: null },
  { id: 'old-0', op: 'server_control', server: 'Main roleplay', state: 'interrupted', started: ago(2800), ended: ago(2790), result: null, error: { code: 'job.interrupted', message: "Kora Panel was closed, or the computer restarted, while this was running for 'Main roleplay', so it did not finish. Look at the server's state, then run it again if you still need it." } },
]

const row = (id: number, mins: number, op: string, server: string | null, who: Pick<AuditRow, 'actor_kind' | 'actor_name' | 'provider' | 'role'>, result: AuditRow['result'], detail: string | null = null): AuditRow => ({ id, at: ago(mins), op, server, ...who, result, detail })
const local = { actor_kind: 'local', actor_name: 'demo', provider: 'local', role: 'admin' } as const
const sam = { actor_kind: 'remote', actor_name: 'sam@example.com', provider: 'tailscale', role: 'admin' } as const
const kim = { actor_kind: 'remote', actor_name: 'kim', provider: 'token', role: 'operator' } as const
const cli = { actor_kind: 'cli', actor_name: 'demo', provider: null, role: null } as const

export const seedAudit = (): AuditRow[] => {
  const base: AuditRow[] = [
    row(1, 4000, 'install_start', 'Main roleplay', local, 'started', 'job a1b2'),
    row(2, 3990, 'install_start', 'Main roleplay', local, 'ok', 'job a1b2'),
    row(3, 2900, 'key_add', null, local, 'ok'),
    row(4, 2800, 'server_control', 'Main roleplay', sam, 'ok'),
    row(5, 1500, 'update_apply', 'Main roleplay', sam, 'started', 'job old-1'),
    row(6, 1498, 'update_apply', 'Main roleplay', sam, 'ok', 'job old-1'),
    row(7, 600, 'server_control', 'Main roleplay', kim, 'ok'),
    row(8, 590, 'server_delete', 'Main roleplay', kim, 'refused', 'HTTP 403'),
    row(9, 300, 'update_apply', 'Main roleplay', cli, 'started'),
    row(10, 298, 'update_apply', 'Main roleplay', cli, 'failed', 'exit code 1'),
  ]
  // enough older rows to need a second page
  const more: AuditRow[] = Array.from({ length: 70 }, (_, i) => row(11 + i, 200 - i, i % 2 ? 'server_control' : 'webhook_set', i % 3 ? 'Staging' : null, i % 4 ? local : sam, 'ok'))
  return [...base, ...more]
}

export interface StoreMock {
  reset: () => void
}

export function installStoreMock(route: Route, live: () => JobSummary[]): StoreMock {
  let jobs = seedJobs()
  let audit = seedAudit()
  route('GET', '/jobs', (_m, _b, q) => {
    const limit = Math.min(Number(q.get('limit') ?? 50) || 50, 300)
    const ids = new Set(live().map((j) => j.id))
    return { jobs: [...live(), ...jobs.filter((j) => !ids.has(j.id))].sort((a, b) => b.started - a.started).slice(0, limit) }
  })
  route('GET', '/audit', (_m, _b, q) => {
    const limit = Math.min(Number(q.get('limit') ?? 100) || 100, 500)
    const before = q.get('before') ? Number(q.get('before')) : Infinity
    const server = q.get('server')
    const actor = (q.get('actor') ?? '').toLowerCase()
    const result = q.get('result')
    const rows = [...audit].sort((a, b) => b.id - a.id).filter((r) => r.id < before && (!server || r.server === server) && (!actor || (r.actor_name ?? '').toLowerCase().includes(actor)) && (!result || r.result === result))
    return { entries: rows.slice(0, limit), more: rows.length > limit, retention_days: 365 }
  })
  return {
    reset: () => {
      jobs = seedJobs()
      audit = seedAudit()
    },
  }
}
