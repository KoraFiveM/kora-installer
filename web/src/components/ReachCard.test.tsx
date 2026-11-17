import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReachLayer, ReachReport } from '@/api/types'
import { jsonResponse } from '@/test/utils'
import { FORWARDING_RISK, ReachCard } from './ReachCard'

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock) })
afterEach(() => vi.unstubAllGlobals())

const layer = (id: ReachLayer['id'], state: ReachLayer['state'], title: string, extra: Partial<ReachLayer> = {}): ReachLayer => ({ id, state, title, detail: '', steps: [], fixes: [], ...extra })

function report(over: Partial<ReachReport> = {}): ReachReport {
  return {
    server: 'prod',
    game_port: 30120,
    layers: [
      layer('running', 'ok', 'The server is running and listening on port 30120 (TCP and UDP)'),
      layer('firewall', 'ok', 'No firewall is blocking this computer'),
      layer('router', 'unknown', 'Your router must forward port 30120', { steps: ['Open your router\'s settings page', 'Add a rule: protocol TCP and UDP, outside/external port 30120, inside/internal port 30120, destination address 192.168.1.50.'], fixes: [{ id: 'enable_forwarding', label: 'Forward port 30120 for me (UPnP)' }] }),
      layer('internet', 'unknown', 'Kora cannot test this from the inside', { steps: ['Ask a friend who is NOT on your home network to start FiveM, press F8 and type: connect 84.12.5.6:30120'], fixes: [{ id: 'allow_cfx_list', label: "Look my server up on Cfx.re's server list" }] }),
    ],
    verdict: 'unknown',
    headline: 'Nothing is wrong that Kora can see, but it cannot prove players can get in: finish with the test below.',
    lan_ip: '192.168.1.50',
    public_ip: '84.12.5.6',
    public_source: 'router',
    behind_nat: true,
    cgnat: false,
    join_code: null,
    connect: { lan: '192.168.1.50:30120', public: '84.12.5.6:30120', cfx: null, command: 'connect 84.12.5.6:30120' },
    forwarding: { enabled: false, status: null },
    contacted: [],
    needs_consent: ['cfx_list'],
    checked_unix: 1,
    consent: { cfx_list: false, ip_echo: false },
    ...over,
  }
}

function setup(handler: (url: string, init?: RequestInit) => Response | Promise<Response>, role: 'admin' | 'operator' = 'admin', props: Partial<React.ComponentProps<typeof ReachCard>> = {}) {
  fetchMock.mockImplementation(async (url: string, init?: RequestInit) => handler(url, init))
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
  qc.setQueryData(['meta'], { version: '0', api: 1, platform: 'linux', ops: [], node: 'n', identity: { name: 'sam', provider: 'local', role }, remote: false })
  return render(<QueryClientProvider client={qc}><ReachCard name="prod" {...props} /></QueryClientProvider>)
}

const calls = (needle: string) => fetchMock.mock.calls.filter((c) => String(c[0]).includes(needle))

describe('ReachCard', () => {
  it('says what it looks at and contacts nothing outside before the person asks', async () => {
    setup(() => jsonResponse(200, report()))
    expect(screen.getByText(/It only looks at this computer and your router/)).toBeInTheDocument()
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('shows one line per layer with exact values to enter in the router, and the addresses to give people', async () => {
    setup(() => jsonResponse(200, report()))
    await userEvent.setup().click(screen.getByRole('button', { name: /check now/i }))
    expect(await screen.findByText(/Your router must forward port 30120/)).toBeInTheDocument()
    const lines = screen.getAllByRole('listitem').filter((li) => li.hasAttribute('data-layer'))
    expect(lines.map((l) => l.getAttribute('data-layer'))).toEqual(['running', 'firewall', 'router', 'internet'])
    expect(lines.map((l) => l.getAttribute('data-state'))).toEqual(['ok', 'ok', 'unknown', 'unknown'])
    expect(screen.getByText(/outside\/external port 30120, inside\/internal port 30120, destination address 192.168.1.50/)).toBeInTheDocument()
    expect(screen.getByLabelText('Address on your own network')).toHaveTextContent('192.168.1.50:30120')
    expect(screen.getByLabelText('Address on the internet')).toHaveTextContent('84.12.5.6:30120')
    expect(calls('/servers/prod/reach')[0][1]).toMatchObject({ method: 'POST' })
  })

  it('asks before the first outside lookup, says what is contacted and what it learns, then remembers the choice', async () => {
    let allowed = false
    setup((url, init) => {
      if (url === '/api/v1/reach/settings' && init?.method === 'PUT') { allowed = true; return jsonResponse(200, { cfx_list: true, ip_echo: false }) }
      return jsonResponse(200, allowed ? report({ layers: [layer('running', 'ok', 'ok'), layer('firewall', 'ok', 'ok'), layer('router', 'ok', 'ok'), layer('internet', 'likely', 'Cfx.re lists your server')], consent: { cfx_list: true, ip_echo: false }, contacted: [{ what: "Cfx.re's public server list (frontend.cfx-services.net)", learned: 'your server\'s join code' }], verdict: 'unknown', needs_consent: [] }) : report())
    })
    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: /check now/i }))
    await user.click(await screen.findByRole('button', { name: /look my server up on cfx\.re/i }))
    expect(calls('/reach/settings')).toHaveLength(0)
    expect(screen.getByText(/This will contact Cfx\.re's public server list \(frontend\.cfx-services\.net\)/)).toBeInTheDocument()
    expect(screen.getByText(/Cfx\.re already knows it/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /allow and check again/i }))
    await waitFor(() => expect(calls('/reach/settings')).toHaveLength(1))
    expect(JSON.parse(String(calls('/reach/settings')[0][1]?.body))).toEqual({ cfx_list: true })
    expect(await screen.findByText(/Cfx\.re lists your server/)).toBeInTheDocument()
    expect(screen.getByText(/This check asked Cfx\.re's public server list/)).toBeInTheDocument()
  })

  it('"Just this once" checks with the lookup but saves nothing', async () => {
    setup(() => jsonResponse(200, report()))
    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: /check now/i }))
    await user.click(await screen.findByRole('button', { name: /look my server up/i }))
    await user.click(screen.getByRole('button', { name: /just this once/i }))
    await waitFor(() => expect(calls('/servers/prod/reach')).toHaveLength(2))
    expect(JSON.parse(String(calls('/servers/prod/reach')[1][1]?.body))).toEqual({ cfx_list: true })
    expect(calls('/reach/settings')).toHaveLength(0)
  })

  it('port forwarding needs a second yes that names the risk and the limits', async () => {
    setup((url, init) => (url.endsWith('/forwarding') && init?.method === 'PUT' ? jsonResponse(200, { enabled: true, status: null }) : jsonResponse(200, report())))
    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: /check now/i }))
    await user.click(await screen.findByRole('button', { name: /forward port 30120 for me/i }))
    expect(calls('/forwarding')).toHaveLength(0)
    const alert = screen.getByRole('alert')
    expect(within(alert).getByText(FORWARDING_RISK)).toBeInTheDocument()
    expect(FORWARDING_RISK).toMatch(/game port only/)
    await user.click(within(alert).getByRole('button', { name: /turn on port forwarding/i }))
    await waitFor(() => expect(calls('/forwarding')).toHaveLength(1))
    expect(JSON.parse(String(calls('/forwarding')[0][1]?.body))).toEqual({ enabled: true })
  })

  it('shows the command to run when the firewall needs administrator rights', async () => {
    const fw = report({ layers: [layer('running', 'ok', 'ok'), layer('firewall', 'problem', 'The firewall (ufw) is on and Kora did not open port 30120', { fixes: [{ id: 'open_firewall', label: 'Open port 30120 in the firewall' }] }), layer('router', 'ok', 'ok'), layer('internet', 'unknown', 'u')], verdict: 'problem', headline: 'Something is in the way: the firewall.' })
    setup((url, init) =>
      url.endsWith('/reach/firewall') && init?.method === 'POST'
        ? jsonResponse(403, { error: { code: 'needs_elevation', message: 'Changing the firewall needs administrator rights. Run this command as administrator, then check again:', command: ['kora', 'reach', 'prod', '--open-firewall'], actions: [{ kind: 'show_command', command: ['kora', 'reach', 'prod', '--open-firewall'], elevated: true }] } })
        : jsonResponse(200, fw),
    )
    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: /check now/i }))
    await user.click(await screen.findByRole('button', { name: /open port 30120 in the firewall/i }))
    expect(await screen.findByText(/kora reach prod --open-firewall/)).toBeInTheDocument()
  })

  it('offers "Start the server" only where the page can start it', async () => {
    const stopped = report({ layers: [layer('running', 'problem', 'The server is not running', { fixes: [{ id: 'start_server', label: 'Start the server' }] }), layer('firewall', 'ok', 'ok'), layer('router', 'ok', 'ok'), layer('internet', 'unknown', 'u')], verdict: 'problem', headline: 'Something is in the way: the server is not running.' })
    const onStart = vi.fn()
    setup(() => jsonResponse(200, stopped), 'admin', { onStart })
    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: /check now/i }))
    await user.click(await screen.findByRole('button', { name: /start the server/i }))
    expect(onStart).toHaveBeenCalled()
  })

  it('operators may check but not change the router or the firewall', async () => {
    setup(() => jsonResponse(200, report()), 'operator')
    await userEvent.setup().click(screen.getByRole('button', { name: /check now/i }))
    const b = await screen.findByRole('button', { name: /forward port 30120 for me/i })
    expect(b).toBeDisabled()
    expect(b).toHaveAttribute('title', expect.stringContaining('admin'))
  })

  it('explains carrier-grade NAT and offers no forwarding button', async () => {
    const cg = report({ cgnat: true, layers: [layer('running', 'ok', 'ok'), layer('firewall', 'ok', 'ok'), layer('router', 'problem', 'Your internet provider shares your public address', { steps: ['Your internet provider shares one public address between many customers (carrier-grade NAT).', 'Run the server at a hosting provider or a cheap VPS instead, or', 'Let friends join over a private network such as Tailscale or ZeroTier'] }), layer('internet', 'problem', 'Not reachable from the internet')], verdict: 'problem', headline: 'Something is in the way: Your internet provider shares your public address.', public_ip: null, connect: { lan: '192.168.1.50:30120', public: null, cfx: null, command: null } })
    setup(() => jsonResponse(200, cg))
    await userEvent.setup().click(screen.getByRole('button', { name: /check now/i }))
    expect(await screen.findByText(/carrier-grade NAT/)).toBeInTheDocument()
    expect(screen.getByText(/Tailscale or ZeroTier/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /forward port/i })).not.toBeInTheDocument()
    expect(screen.queryByLabelText('Address on the internet')).not.toBeInTheDocument()
  })
})

describe('MoreAddresses', () => {
  it('shows the public address and the Cfx.re link a check found, and nothing before one', async () => {
    const { MoreAddresses } = await import('./ReachCard')
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const ui = (
      <QueryClientProvider client={qc}>
        <MoreAddresses name="prod" />
      </QueryClientProvider>
    )
    const { container, rerender } = render(ui)
    expect(container).toBeEmptyDOMElement()
    qc.setQueryData(['reach', 'prod'], report({ connect: { lan: '192.168.1.50:30120', public: '84.12.5.6:30120', cfx: 'https://cfx.re/join/abc123', command: 'connect 84.12.5.6:30120' } }))
    rerender(ui)
    expect(await screen.findByLabelText('Internet address')).toHaveTextContent('84.12.5.6:30120')
    expect(screen.getByLabelText('Cfx.re join link')).toHaveTextContent('https://cfx.re/join/abc123')
  })
})
