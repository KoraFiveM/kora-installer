// Seed data for the mock backend. Only bundled when VITE_MOCK=1 (or in `npm run dev`).
import type { Badge, Catalog, CatalogResource, Group, Kind, LogFile, Mode, ServerStatus, ServerSummary, SnapshotInfo } from '../types'
import type { ScenarioName } from './scenarios'

const now = Math.floor(Date.now() / 1000)
const DAY = 86400

const FIXED_NOW = 1_780_000_000 // fleet servers use a fixed clock so screenshots repeat

/** ~30 servers: every mode x status x badge, long names, high ports, one adopted (not managed) server. */
export function seedFleet(): ServerSummary[] {
  const modes: Mode[] = ['production', 'development']
  const statuses: ServerStatus[] = ['running', 'stopped', 'missing']
  const badges: [string, Badge | null][] = [
    ['ok', { kind: 'up_to_date' }],
    ['upd', { kind: 'updates', count: 1 }],
    ['attn', { kind: 'needs_attention' }],
    ['new', { kind: 'new_release' }],
    ['none', null],
  ]
  const out: ServerSummary[] = []
  let i = 0
  for (const mode of modes)
    for (const status of statuses)
      for (const [slug, badge] of badges) {
        const long = i % 5 === 0
        const base = `${mode === 'production' ? 'prod' : 'dev'}-${status}-${slug}`
        const name = long ? `${String(i).padStart(2, '0')}-${base}-the-very-long-community-roleplay-server`.slice(0, 32).replace(/-$/, 'x') : base
        const high = i % 7 === 3
        const b = badge?.kind === 'updates' ? { kind: 'updates' as const, count: 1 + ((i * 7) % 23) } : badge
        out.push({
          name,
          dir: `/srv/fivem/${name}`,
          mode,
          ports: high ? { game: 64000 + i, txadmin: 65000 + i } : { game: 30120 + i, txadmin: 40120 + i },
          created_unix: FIXED_NOW - (i + 1) * 3 * DAY,
          status,
          badge: b,
          managed: true,
          connect: null,
        })
        i++
      }
  out.push({ name: 'adopted-existing-server', dir: '/home/sam/games/adopted-existing-server', mode: 'production', ports: { game: 30190, txadmin: 40190 }, created_unix: FIXED_NOW - 400 * DAY, status: 'stopped', badge: null, managed: false, connect: null })
  return out
}

export function seedServers(scenario: ScenarioName = 'default'): ServerSummary[] {
  if (scenario === 'empty') return []
  if (scenario === 'fleet') return seedFleet()
  return [
    { name: 'kora-live', dir: '/srv/fivem/kora-live', mode: 'production', ports: { game: 30120, txadmin: 40120 }, created_unix: now - 40 * DAY, status: 'running', badge: { kind: 'up_to_date' }, managed: true, connect: null },
    { name: 'kora-dev', dir: '/srv/fivem/kora-dev', mode: 'development', ports: { game: 30121, txadmin: 40121 }, created_unix: now - 12 * DAY, status: 'stopped', badge: { kind: 'updates', count: 2 }, managed: true, connect: null },
    { name: 'kora-old', dir: '/srv/fivem/kora-old', mode: 'production', ports: { game: 30122, txadmin: 40122 }, created_unix: now - 200 * DAY, status: 'missing', badge: null, managed: true, connect: null },
  ]
}

function res(id: string, group: Group, kind: Kind, extra: Partial<CatalogResource> = {}): CatalogResource {
  return {
    id,
    title: extra.title ?? id.replace(/^kora_/, '').replace(/_/g, ' ').replace(/^./, (c) => c.toUpperCase()),
    repository: `https://github.com/KoraFiveM/${id}`,
    path: id,
    group,
    kind,
    branch: 'main',
    latest: '1.4.0',
    versions: [{ version: '1.4.0', tag: 'v1.4.0', commit: 'a1b2c3d4e5f6' }],
    depends_on: [],
    ...extra,
  }
}

export function seedCatalog(): Catalog {
  return {
    schema_version: 3,
    creators: [
      { id: 'northside', name: 'Northside Scripts', url: 'https://example.invalid/northside', description: 'Community maps and jobs.' },
    ],
    resources: [
      res('oxmysql', 'pre', 'dependency', { title: 'oxmysql (database)', description: 'Connects resources to the database.', creator: undefined }),
      res('ox_lib', 'pre', 'dependency', { title: 'ox_lib (helpers)', description: 'Shared helper library used by many resources.' }),
      res('kora_core', 'kora', 'kora', { description: 'The Kora framework core.', depends_on: ['oxmysql', 'ox_lib'] }),
      res('kora_inventory', 'kora', 'kora', { description: 'Player inventory and items.', depends_on: ['kora_core'] }),
      res('kora_jobs', 'kora', 'kora', { description: 'Jobs, duty and payroll.', depends_on: ['kora_core'] }),
      res('kora_phone', 'kora', 'kora', { description: 'In-game phone.', depends_on: ['kora_core', 'kora_inventory'] }),
      res('kora_console', 'kora', 'internal', { description: 'Console helper.' }),
      res('chat', 'cfx-default', 'optional', { title: 'Chat', description: 'The standard Cfx.re chat.' }),
      res('mapmanager', 'cfx-default', 'optional', { title: 'Map manager', description: 'Loads maps for game modes.' }),
      res('legacy_bridge', 'compat', 'optional', { title: 'Legacy framework compatibility', description: 'Lets scripts written for older frameworks run on Kora.', depends_on: ['kora_core'] }),
      res('esx_compat', 'compat', 'optional', { title: 'ESX compatibility', description: 'Runs ESX-style resources on Kora.', depends_on: ['kora_core'] }),
      res('target_bridge', 'bridges', 'optional', { title: 'Target bridge', description: 'Connects third-party targeting scripts.', depends_on: ['kora_core'] }),
      res('banking_bridge', 'bridges', 'optional', { title: 'Banking bridge', description: 'Connects third-party banking scripts.', depends_on: ['kora_core'] }),
      res('ns_cityhall', 'creators', 'optional', { title: 'Northside City Hall', description: 'A city hall interior and jobs.', creator: 'northside', depends_on: ['kora_core', 'kora_jobs'] }),
      res('ns_garage', 'creators', 'optional', { title: 'Northside Garage', description: 'A public garage map.', creator: 'northside', depends_on: ['kora_core'] }),
      res('kora_devtools', 'kora', 'optional', { description: 'Tools for building resources.', dev_only: true, depends_on: ['kora_core'] }),
    ],
  }
}

export const INSTALLED: Record<string, string[]> = {
  'kora-live': ['oxmysql', 'ox_lib', 'kora_core', 'kora_inventory', 'kora_jobs', 'chat', 'mapmanager', 'kora_console'],
  'kora-dev': ['oxmysql', 'ox_lib', 'kora_core', 'kora_inventory', 'kora_jobs', 'kora_phone', 'chat'],
  'kora-old': ['oxmysql', 'ox_lib', 'kora_core'],
}

export function seedSnapshots(): Record<string, SnapshotInfo[]> {
  return {
    'kora-live': [{ id: 'snap-20260301-0412', created_unix: now - 9 * DAY, resources: ['kora_core', 'kora_jobs'] }],
    'kora-dev': [
      { id: 'snap-20260310-0400', created_unix: now - 2 * DAY, resources: ['kora_inventory'] },
      { id: 'snap-20260308-0400', created_unix: now - 4 * DAY, resources: ['kora_core', 'kora_phone'] },
    ],
    'kora-old': [],
  }
}

export const LOG_FILES: LogFile[] = [
  { name: 'latest.log', size: 18_204, modified_unix: now - 30 },
  { name: 'txadmin.log', size: 92_011, modified_unix: now - 600 },
  { name: 'previous.log', size: 240_880, modified_unix: now - 2 * DAY },
]

export function fakeLogLines(server: string, file: string, count: number): string[] {
  const out: string[] = []
  for (let i = 0; i < count; i++) {
    const t = new Date(Date.now() - (count - i) * 4000).toISOString().replace('T', ' ').slice(0, 19)
    const pick = i % 6
    out.push(
      pick === 0
        ? `[${t}] [${file}] ${server}: Started resource kora_core`
        : pick === 1
          ? `[${t}] [${file}] ${server}: Player connected (id ${100 + i})`
          : pick === 2
            ? `[${t}] [${file}] ${server}: Database pool ready (8 connections)`
            : pick === 3
              ? `[${t}] [${file}] ${server}: Autosave finished in 41 ms`
              : pick === 4
                ? `[${t}] [${file}] ${server}: Player disconnected (id ${100 + i - 1}): quit`
                : `[${t}] [${file}] ${server}: Server thread heartbeat ok`,
    )
  }
  return out
}

export const CONSOLE_LINES = [
  'Autosave finished in 38 ms',
  'Player "Sam" connected (id 12)',
  'kora_jobs: payroll run complete (3 players)',
  'Server thread heartbeat ok',
  'Player "Sam" disconnected: quit',
  'oxmysql: 4 queries in the last minute, slowest 6 ms',
]
