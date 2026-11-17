// Mock of the reachability check, port forwarding and the txAdmin ticket (kept apart from the main mock).
// `installReachMock` is called once from mock/index.ts.
import type * as T from '../types'
import type { ReachConsent, ReachReport } from '../reach'

type Body = Record<string, unknown>
type Route = (method: string, pattern: string, h: (m: RegExpMatchArray, body: Body, query: URLSearchParams) => unknown | Promise<unknown>) => void

export interface ReachMock {
  reset: () => void
}

export function installReachMock(route: Route, servers: () => T.ServerSummary[]): ReachMock {
  let consent: ReachConsent = { cfx_list: false, ip_echo: false }
  let forwarding: Record<string, boolean> = {}
  let firewallOpen: Record<string, boolean> = {}

  const report = (name: string, once: Partial<ReachConsent>): ReachReport => {
    const s = servers().find((x) => x.name === name)
    const port = s?.ports.game ?? 30120
    const running = s?.status === 'running'
    const allowCfx = once.cfx_list ?? consent.cfx_list
    const allowEcho = once.ip_echo ?? consent.ip_echo
    const fwd = forwarding[name] ?? false
    const publicIp = '84.12.5.6'
    const layers: ReachReport['layers'] = [
      running
        ? { id: 'running', state: 'ok', title: `The server is running and listening on port ${port} (TCP and UDP)`, detail: 'This computer accepts connections on the game port.', steps: [], fixes: [] }
        : { id: 'running', state: 'problem', title: 'The server is not running', detail: 'Nobody can join a server that is switched off.', steps: [], fixes: [{ id: 'start_server', label: 'Start the server' }] },
      firewallOpen[name]
        ? { id: 'firewall', state: 'ok', title: `The firewall (ufw) lets port ${port} in`, detail: 'Kora opened this server\'s port.', steps: [], fixes: [] }
        : { id: 'firewall', state: 'problem', title: `The firewall (ufw) is on and Kora did not open port ${port}`, detail: `Unless you opened port ${port} yourself (TCP and UDP), players are blocked here.`, steps: [`sudo ufw allow ${port}/tcp && sudo ufw allow ${port}/udp`], fixes: [{ id: 'open_firewall', label: `Open port ${port} in the firewall` }] },
      fwd
        ? { id: 'router', state: 'ok', title: `Port ${port} is forwarded by your router`, detail: `Your router (UPnP) forwards port ${port} (TCP and UDP) while the server runs. Your public address is ${publicIp}.`, steps: [], fixes: [] }
        : {
            id: 'router',
            state: 'unknown',
            title: `Your router must forward port ${port}`,
            detail: `This computer is behind a router, so players on the internet only get in if the router sends port ${port} (TCP and UDP) here. Kora cannot see the router's rules. Your public address is ${publicIp}.`,
            steps: [
              "Open your router's settings page (often http://192.168.0.1 or http://192.168.1.1; the address and password are usually on a sticker on the router).",
              'Find "Port forwarding" (sometimes "Virtual server", "NAT" or "Applications").',
              `Add a rule: protocol TCP and UDP (both; if there is only one choice make two rules), outside/external port ${port}, inside/internal port ${port}, destination address 192.168.1.50.`,
              'Save, then press "Check again" here.',
            ],
            fixes: [{ id: 'enable_forwarding', label: `Forward port ${port} for me (UPnP)` }],
          },
      allowCfx
        ? { id: 'internet', state: 'likely', title: 'Cfx.re lists your server', detail: `Cfx.re's server list shows it as cfx.re/join/abc123 (address ${publicIp}:${port}). That is a good sign, but only a real player joining proves that the game's UDP port works too.`, steps: [`Ask a friend who is NOT on your home network to start FiveM, press F8 and type: connect ${publicIp}:${port}`], fixes: [] }
        : {
            id: 'internet',
            state: 'unknown',
            title: 'Kora cannot test this from the inside',
            detail: "Only a computer outside your network can tell whether it gets through. Kora can look your server up on Cfx.re's public server list, which already knows it (it learns nothing new). It shows whether you are listed and at which address.",
            steps: [`Ask a friend who is NOT on your home network to start FiveM, press F8 and type: connect ${publicIp}:${port}`],
            fixes: [{ id: 'allow_cfx_list', label: "Look my server up on Cfx.re's server list" }],
          },
    ]
    const problem = layers.find((l) => l.state === 'problem')
    const verdict = problem ? 'problem' : layers.every((l) => l.state === 'ok') ? 'ok' : 'unknown'
    return {
      server: name,
      game_port: port,
      layers,
      verdict,
      headline: problem ? `Something is in the way: ${problem.title}.` : 'Nothing is wrong that Kora can see, but it cannot prove players can get in: finish with the test below.',
      lan_ip: '192.168.1.50',
      public_ip: publicIp,
      public_source: 'router',
      behind_nat: true,
      cgnat: false,
      join_code: allowCfx ? 'abc123' : null,
      connect: { lan: `192.168.1.50:${port}`, public: `${publicIp}:${port}`, cfx: allowCfx ? 'https://cfx.re/join/abc123' : null, command: `connect ${publicIp}:${port}` },
      forwarding: { enabled: fwd, status: fwd ? { at: Math.floor(Date.now() / 1000), ok: true, via: 'UPnP', message: `Your router (UPnP) forwards port ${port} (TCP and UDP) while the server runs.`, external_ip: publicIp, permanent: false } : null },
      contacted: allowCfx || allowEcho ? [...(allowCfx ? [{ what: "Cfx.re's public server list (frontend.cfx-services.net)", learned: 'your server\'s join code (it already knows it)' }] : [])] : [],
      needs_consent: allowCfx ? [] : ['cfx_list'],
      checked_unix: Math.floor(Date.now() / 1000),
      consent,
    }
  }

  route('POST', '/servers/:s/reach', (m, body) => report(decodeURIComponent(m[1]), body as Partial<ReachConsent>))
  route('GET', '/reach/settings', () => consent)
  route('PUT', '/reach/settings', (_m, body) => {
    consent = { cfx_list: typeof body.cfx_list === 'boolean' ? body.cfx_list : consent.cfx_list, ip_echo: typeof body.ip_echo === 'boolean' ? body.ip_echo : consent.ip_echo }
    return consent
  })
  route('PUT', '/servers/:s/forwarding', (m, body) => {
    forwarding[decodeURIComponent(m[1])] = body.enabled === true
    return { enabled: body.enabled === true, status: null }
  })
  route('POST', '/servers/:s/reach/firewall', (m) => {
    firewallOpen[decodeURIComponent(m[1])] = true
    return { notes: ['Opened the game port (TCP and UDP) in the firewall.'] }
  })
  route('POST', '/servers/:s/txadmin/ticket', (m) => {
    const name = decodeURIComponent(m[1])
    const s = servers().find((x) => x.name === name)
    const port = (s?.ports.txadmin ?? 40120) + 10000
    return { url: `${window.location.protocol}//${window.location.hostname}:${port}/__kora/enter?t=demo`, port, expires_in: 60, username: 'kora-admin' }
  })

  return {
    reset: () => {
      consent = { cfx_list: false, ip_echo: false }
      forwarding = {}
      firewallOpen = {}
    },
  }
}
