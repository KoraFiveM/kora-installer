// Mock data and routes for the fleet page (kept apart from the main mock so edits do not collide).
// `installHubMock` is called once from mock/index.ts with the mock's own helpers.
import type * as T from '../types'

type Body = Record<string, unknown>
type Route = (method: string, pattern: string, h: (m: RegExpMatchArray, body: Body, query: URLSearchParams) => unknown | Promise<unknown>) => void
type FailCtor = new (status: number, code: string, message: string) => Error
type StartJob = (op: string, server: string | null, run: (emit: (e: unknown) => void) => Promise<unknown>) => T.JobStarted

interface MockNode {
  node: T.HubNode
  servers: T.ServerSummary[]
}

const now = Math.floor(Date.now() / 1000)
const ALL_OPS = ['meta', 'servers_list', 'server_get', 'server_control', 'jobs_get', 'update_preview']
const srv = (name: string, status: T.ServerStatus, game: number): T.ServerSummary => ({ name, dir: `/srv/${name}`, mode: 'production', ports: { game, txadmin: game + 10000 }, created_unix: now - 86400 * 30, status, badge: { kind: 'up_to_date' }, managed: true })

function seed(): MockNode[] {
  const base = { token_set: true, insecure: false, warning: null, reachable: true, platform: 'linux', last_error: null }
  return [
    { node: { ...base, id: 'n-garage', name: 'Garage box', url: 'http://192.168.1.20:47120', version: '0.4.0', api: 1, ops: ALL_OPS, server_count: 2, can_control: true }, servers: [srv('garage-rp', 'running', 30130), srv('garage-test', 'stopped', 30131)] },
    { node: { ...base, id: 'n-old', name: 'Old laptop', url: 'http://100.64.3.9:47120', version: '0.3.1', api: 1, ops: ['meta', 'servers_list', 'server_get'], server_count: 1, can_control: false }, servers: [srv('laptop-dev', 'running', 30140)] },
    { node: { ...base, id: 'n-away', name: 'Cloud host', url: 'https://kora.example.com', reachable: false, version: null, api: null, ops: [], server_count: null, can_control: false, last_error: 'The node did not answer in time.' }, servers: [] },
  ]
}

export function installHubMock(route: Route, startJob: StartJob, Fail: FailCtor, local: () => T.ServerSummary[]): void {
  const nodes = seed()
  let next = 1
  const find = (id: string) => nodes.find((n) => n.node.id === id) ?? Promise.reject(new Fail(404, 'not_found', `There is no node '${id}'.`))
  const tag = (n: MockNode): T.FleetServer[] => n.servers.map((s) => ({ ...s, node_id: n.node.id, node_name: n.node.name, actions: n.node.can_control ? ['start', 'stop', 'restart'] : [] }))
  const views = () => nodes.map((n) => ({ ...n.node, server_count: n.node.reachable ? n.servers.length : null }))

  route('GET', '/nodes', () => ({ nodes: views() }))
  route('POST', '/nodes', (_m, b) => {
    const url = String(b.url ?? '')
    if (!/^https?:\/\/[^/\s@]+/.test(url)) throw new Fail(400, 'bad_request', 'The address must start with http:// or https://.')
    if (!String(b.name ?? '').trim()) throw new Fail(400, 'bad_request', 'Give the node a name of up to 64 characters.')
    if (!String(b.token ?? '').trim()) throw new Fail(400, 'bad_request', 'Enter the token from that panel.')
    const insecure = url.startsWith('http://') && !/\/\/(localhost|127\.|10\.|192\.168\.|100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.|172\.(1[6-9]|2\d|3[01])\.)/.test(url)
    const node: T.HubNode = { id: `n-${next++}`, name: String(b.name).trim(), url, token_set: true, insecure, warning: insecure ? 'This address uses plain http:// over the internet, so the token and your server details can be read on the way. Use https:// or Tailscale.' : null, reachable: true, version: '0.4.0', api: 1, platform: 'linux', ops: ALL_OPS, server_count: 1, can_control: true, last_error: null }
    nodes.push({ node, servers: [srv(`${node.name.toLowerCase().replace(/\W+/g, '-')}-main`, 'running', 30150)] })
    return { node }
  })
  route('DELETE', '/nodes/{id}', async (m) => {
    const n = await find(decodeURIComponent(m[1]))
    nodes.splice(nodes.indexOf(n), 1)
  })
  route('GET', '/nodes/{id}/servers', async (m) => {
    const n = await find(decodeURIComponent(m[1]))
    if (!n.node.reachable) throw new Fail(502, 'node_unreachable', n.node.last_error ?? 'The node could not be reached.')
    return { servers: tag(n) }
  })
  route('POST', '/nodes/{id}/servers/{s}/control', async (m, b) => {
    const n = await find(decodeURIComponent(m[1]))
    if (!n.node.can_control) throw new Fail(422, 'node_unsupported', `That node (Kora ${n.node.version ?? '?'}) does not support this yet. Update it to use this from here.`)
    const s = n.servers.find((x) => x.name === decodeURIComponent(m[2]))
    if (!s) throw new Fail(404, 'not_found', `There is no server named '${decodeURIComponent(m[2])}'.`)
    const action = String(b.action)
    return startJob('node_server_control', `node:${n.node.id}:${s.name}`, async (emit) => {
      emit({ type: 'status', message: `Asking ${n.node.name} to ${action}` })
      await new Promise((r) => setTimeout(r, 800))
      s.status = action === 'stop' ? 'stopped' : 'running'
      return { status: s.status }
    })
  })
  route('GET', '/fleet', () => ({
    servers: [...local().map((s) => ({ ...s, node_id: null, node_name: 'This panel', actions: ['start', 'stop', 'restart'] })), ...nodes.flatMap(tag)],
    nodes: views(),
    partial: nodes.some((n) => !n.node.reachable),
  }))
}
