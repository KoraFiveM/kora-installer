// Mock data for the monitoring routes. Hooked up from index.ts with one `installMonitorMock(...)` call.
import type * as T from '../types'

type Handler = (m: RegExpMatchArray, body: Record<string, unknown>, query: URLSearchParams) => unknown
interface Hooks {
  route: (method: string, pattern: string, h: Handler) => void
  servers: T.ServerSummary[]
  fail: (status: number, code: string, message: string) => Error
}

const SECONDS: Record<T.MetricsRange, number> = { '1h': 3600, '6h': 21600, '24h': 86400 }

/** A smooth, repeatable curve so the charts look alive and tests are stable. */
function wave(i: number, period: number, seed: number): number {
  return 0.5 + 0.35 * Math.sin((i / period) * 2 * Math.PI + seed) + 0.1 * Math.sin(i * 0.9 + seed * 3)
}

function sample(t: number, i: number, up: boolean, seed: number): T.MetricSample {
  if (!up) return { t, up: false, game_up: false, txadmin_up: false, sys_ram_pct: 41, disk_free_mb: 61_000, disk_free_pct: 24 }
  return {
    t,
    up: true,
    players: Math.max(0, Math.round(wave(i, 90, seed) * 40)),
    max_players: 48,
    cpu: Math.round(wave(i, 70, seed + 1) * 300) / 10,
    ram_mb: Math.round(2200 + wave(i, 200, seed + 2) * 900),
    sys_ram_pct: Math.round(wave(i, 300, seed + 3) * 300 + 300) / 10,
    disk_free_mb: 61_000,
    disk_free_pct: 24,
    game_up: true,
    txadmin_up: true,
  }
}

export function installMonitorMock({ route, servers, fail }: Hooks): void {
  const now = () => Math.floor(Date.now() / 1000)
  const nameOf = (m: RegExpMatchArray) => {
    const n = decodeURIComponent(m[1])
    if (!servers.some((s) => s.name === n)) throw fail(404, 'not_found', `There is no server called "${n}".`)
    return n
  }
  const settings: Record<string, T.AlertSettings & { url: string | null }> = {}
  const settingsOf = (n: string) =>
    (settings[n] ??= {
      enabled: true,
      rules: { stopped: true, recovered: true, disk: true, ram: true, updates: true },
      disk_min_gb: 5,
      disk_min_percent: 5,
      ram_max_percent: 90,
      discord: true,
      webhook: { configured: false, redacted: null },
      email: { supported: false },
      url: null,
    })
  const view = (n: string): T.AlertSettings => {
    const { url: _url, ...rest } = settingsOf(n)
    return rest
  }
  const alerts: T.PanelAlert[] = [
    { id: 1, server: 'kora-dev', kind: 'stopped', tone: 'danger', title: 'Server stopped', message: 'The server stopped without being asked to. It may have crashed.', raised: now() - 1800, resolved: null, acked: false },
    { id: 2, server: 'kora-live', kind: 'disk_low', tone: 'warning', title: 'Disk space is low', message: "Only 4.2 GB (4%) is free on the server's disk.", raised: now() - 600, resolved: null, acked: false },
  ]

  route('GET', '/servers/{s}/metrics', (m, _b, q) => {
    const name = nameOf(m)
    const range = (q.get('range') ?? '1h') as T.MetricsRange
    if (!(range in SECONDS)) throw fail(400, 'bad_request', 'The range must be 1h, 6h or 24h.')
    const srv = servers.find((s) => s.name === name)
    const up = srv?.status === 'running'
    const seed = name.length
    const points = range === '1h' ? 240 : 360
    const step = SECONDS[range] / points
    const t1 = now()
    const samples = Array.from({ length: points }, (_, i) => sample(Math.round(t1 - (points - 1 - i) * step), i, up, seed))
    return { samples, latest: samples[samples.length - 1] ?? null }
  })
  route('GET', '/alerts', () => ({ alerts: [...alerts].reverse() }))
  route('POST', '/alerts/{id}/ack', (m) => {
    const a = alerts.find((x) => x.id === Number(m[1]))
    if (!a) throw fail(404, 'not_found', 'There is no alert with that number.')
    a.acked = true
    return undefined
  })
  route('GET', '/servers/{s}/alerts', (m) => view(nameOf(m)))
  route('PUT', '/servers/{s}/alerts', (m, b) => {
    const n = nameOf(m)
    const s = settingsOf(n)
    const u = b as T.AlertSettingsUpdate
    const gb = u.disk_min_gb ?? s.disk_min_gb
    const dp = u.disk_min_percent ?? s.disk_min_percent
    const rp = u.ram_max_percent ?? s.ram_max_percent
    if (!(gb >= 0) || !(dp >= 0 && dp <= 100) || !(rp >= 1 && rp <= 100)) throw fail(400, 'bad_request', 'One of the limits is outside its allowed range.')
    if (u.webhook_url && !/^https?:\/\/[^\s/]+/.test(u.webhook_url)) throw fail(400, 'bad_request', 'That is not a usable web address (it must start with http:// or https://).')
    Object.assign(s, { enabled: u.enabled ?? s.enabled, discord: u.discord ?? s.discord, disk_min_gb: gb, disk_min_percent: dp, ram_max_percent: rp })
    s.rules = { ...s.rules, ...u.rules }
    if (u.webhook_url !== undefined) {
      s.url = u.webhook_url || null
      s.webhook = s.url ? { configured: true, redacted: `${new URL(s.url).origin}/***` } : { configured: false, redacted: null }
    }
    return view(n)
  })
}
