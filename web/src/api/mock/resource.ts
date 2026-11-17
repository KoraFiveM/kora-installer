// Mock data and routes for the resource page and its "restart needed" notice (kept apart from the main mock).
import type * as T from '../types'

type Body = Record<string, unknown>
type Route = (method: string, pattern: string, h: (m: RegExpMatchArray, body: Body, query: URLSearchParams) => unknown | Promise<unknown>) => void
type FailCtor = new (status: number, code: string, message: string) => Error

export interface ResourceDeps {
  find: (name: string) => T.ServerSummary
  installed: (server: string) => string[]
  consoleEnabled: (server: string) => boolean
  /** The git state of a resource (what the branches mock says). */
  branchRow: (server: string, id: string) => T.ResourceBranch
}

const NOTICE = 'Restart needed: the running server still has the old version.'

export function installResourceMock(route: Route, Fail: FailCtor, d: ResourceDeps) {
  // server/resource -> why
  const pending = new Map<string, string>()
  const key = (s: string, r: string) => `${s}/${r}`

  route('GET', '/servers/{s}/resources/{name}', (m) => {
    const s = d.find(decodeURIComponent(m[1]))
    const id = decodeURIComponent(m[2])
    if (!d.installed(s.name).includes(id)) throw new Fail(404, 'not_found', `'${id}' is not installed on this server.`)
    const running = s.status === 'running'
    const why = pending.get(key(s.name, id))
    const page: T.ResourcePage = {
      id,
      folder: id,
      repository: `https://github.com/KoraFiveM/${id}.git`,
      default_branch: 'dev',
      pinned_commit: s.mode === 'production' ? 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678' : undefined,
      third_party: false,
      branch: d.branchRow(s.name, id),
      restart: why !== undefined && running ? { needed: true, why, changed_unix: Math.floor(Date.now() / 1000), notice: NOTICE } : { needed: false },
      server: { name: s.name, mode: s.mode, running, console_enabled: d.consoleEnabled(s.name), console_waiting_for_restart: false },
    }
    return page
  })

  return {
    markChanged: (server: string, ids: string[], why: string) => ids.forEach((id) => pending.set(key(server, id), why)),
    resourceRestarted: (server: string, id: string) => pending.delete(key(server, id)),
    serverRestarted: (server: string) => [...pending.keys()].filter((k) => k.startsWith(`${server}/`)).forEach((k) => pending.delete(k)),
    reset: () => pending.clear(),
  }
}
