import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MonitoringTab } from './MonitoringTab'
import { AlertsBanner } from '@/components/AlertsBanner'
import { niceCeil } from '@/components/LineChart'
import type { AlertSettings, MetricSample, PanelAlert, ServerDetail } from '@/api/types'
import { jsonResponse, renderApp } from '@/test/utils'

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock) })
afterEach(() => vi.unstubAllGlobals())

const server = { name: 'prod', dir: '/srv/prod', mode: 'production', source: 'clone', versions: 'released', ports: { game: 30120, txadmin: 40120 }, created_unix: 0, status: 'running', badge: null, managed: true, running: true, game_listening: true, txadmin_up: true, status_detail: { running: true, game_listening: true, txadmin_up: true }, connect: null, console_enabled: false } satisfies ServerDetail

const up = (t: number, players: number): MetricSample => ({ t, up: true, players, max_players: 48, cpu: 12.5, ram_mb: 2048, sys_ram_pct: 55, disk_free_mb: 51200, disk_free_pct: 25, game_up: true, txadmin_up: true })
const settings: AlertSettings = {
  enabled: true,
  rules: { stopped: true, recovered: true, disk: true, ram: true, updates: true, machine: true },
  disk_min_gb: 5,
  disk_min_percent: 5,
  ram_max_percent: 90,
  discord: true,
  webhook: { configured: true, redacted: 'https://hooks.example.test/***' },
  email: { supported: false },
}

function serve(extra: (url: string, init?: RequestInit) => Response | undefined = () => undefined) {
  fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
    const own = extra(url, init)
    if (own) return own
    if (url.startsWith('/api/v1/servers/prod/metrics')) return jsonResponse(200, { samples: [up(1000, 5), up(1015, 9)], latest: up(1015, 9) })
    if (url === '/api/v1/servers/prod/alerts' && (init?.method ?? 'GET') === 'GET') return jsonResponse(200, settings)
    return jsonResponse(404, { error: { code: 'not_found', message: 'no' } })
  })
}

describe('MonitoringTab', () => {
  it('shows the current numbers and a chart for each measurement', async () => {
    serve()
    renderApp(<MonitoringTab server={server} />)
    expect(await screen.findByText('of 48')).toBeInTheDocument()
    expect(screen.getByText('9')).toBeInTheDocument()
    expect(screen.getByText('12.5%')).toBeInTheDocument()
    expect(screen.getByText('51.2 GB')).toBeInTheDocument()
    expect(screen.getAllByRole('img').map((i) => i.getAttribute('aria-label'))).toEqual([
      expect.stringContaining('Players: now 9, highest 9'),
      expect.stringContaining('Server CPU'),
      expect.stringContaining('Server memory: now 2 GB'),
    ])
  })

  it('asks for the range that was picked', async () => {
    serve()
    const user = userEvent.setup()
    renderApp(<MonitoringTab server={server} />)
    await screen.findByText('of 48')
    await user.click(screen.getByRole('button', { name: 'Last 24 hours' }))
    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => String(u).includes('range=24h'))).toBe(true))
    expect(screen.getByRole('button', { name: 'Last 24 hours' })).toHaveAttribute('aria-pressed', 'true')
  })

  it('says so when nothing has been measured yet', async () => {
    serve((url) => (url.includes('/metrics') ? jsonResponse(200, { samples: [], latest: null }) : undefined))
    renderApp(<MonitoringTab server={server} />)
    expect(await screen.findByText(/Nothing has been measured yet/)).toBeInTheDocument()
    expect(screen.getAllByText('no data')).toHaveLength(3)
  })

  it('saves the settings, never sends an unchanged address and never shows the saved one', async () => {
    serve((url, init) => (url === '/api/v1/servers/prod/alerts' && init?.method === 'PUT' ? jsonResponse(200, settings) : undefined))
    const user = userEvent.setup()
    renderApp(<MonitoringTab server={server} />)
    const disk = await screen.findByLabelText('Alert below (GB free)')
    expect(screen.getByText('https://hooks.example.test/***')).toBeInTheDocument()
    await user.clear(disk)
    await user.type(disk, '20')
    await user.click(screen.getByLabelText('Memory is nearly full'))
    await user.click(screen.getByRole('button', { name: 'Save alert settings' }))
    await waitFor(() => expect(screen.getByText('Alert settings saved.')).toBeInTheDocument())
    const put = fetchMock.mock.calls.find(([, i]) => i?.method === 'PUT')!
    const body = JSON.parse(put[1].body)
    expect(body).toMatchObject({ disk_min_gb: 20, rules: { ram: false, disk: true }, enabled: true })
    expect('webhook_url' in body).toBe(false)
  })

  it('sends a new address, and an empty one to remove it', async () => {
    serve((url, init) => (url === '/api/v1/servers/prod/alerts' && init?.method === 'PUT' ? jsonResponse(200, settings) : undefined))
    const user = userEvent.setup()
    renderApp(<MonitoringTab server={server} />)
    await user.type(await screen.findByLabelText('Replace the web address'), 'https://example.test/hook')
    await user.click(screen.getByRole('button', { name: 'Save alert settings' }))
    await waitFor(() => expect(screen.getByText('Alert settings saved.')).toBeInTheDocument())
    expect(JSON.parse(fetchMock.mock.calls.find(([, i]) => i?.method === 'PUT')![1].body).webhook_url).toBe('https://example.test/hook')
    fetchMock.mockClear()
    await user.click(screen.getByLabelText('Remove the saved web address'))
    await user.click(screen.getByRole('button', { name: 'Save alert settings' }))
    await waitFor(() => expect(fetchMock).toHaveBeenCalled())
    expect(JSON.parse(fetchMock.mock.calls.find(([, i]) => i?.method === 'PUT')![1].body).webhook_url).toBe('')
  })

  it('blocks a limit that is out of range', async () => {
    serve()
    const user = userEvent.setup()
    renderApp(<MonitoringTab server={server} />)
    const ram = await screen.findByLabelText('Alert above (% of the machine in use)')
    await user.clear(ram)
    await user.type(ram, '500')
    expect(screen.getByRole('alert')).toHaveTextContent('between 1 and 100')
    expect(screen.getByRole('button', { name: 'Save alert settings' })).toBeDisabled()
  })
})

describe('AlertsBanner', () => {
  const alert = (over: Partial<PanelAlert>): PanelAlert => ({ id: 1, server: 'prod', kind: 'stopped', tone: 'danger', title: 'Server stopped', message: 'It may have crashed.', raised: 1, resolved: null, acked: false, ...over })

  it('shows open alerts with a link to the server, and acknowledging removes them', async () => {
    let list = [alert({}), alert({ id: 2, kind: 'disk_low', title: 'Disk space is low', tone: 'warning', message: 'Low.', acked: true }), alert({ id: 3, resolved: 5 })]
    fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
      if (url === '/api/v1/alerts/1/ack') {
        list = list.map((a) => (a.id === 1 ? { ...a, acked: true } : a))
        expect(init?.method).toBe('POST')
        return jsonResponse(204)
      }
      return jsonResponse(200, { alerts: list })
    })
    const user = userEvent.setup()
    renderApp(<AlertsBanner />)
    expect(await screen.findByText('It may have crashed.')).toBeInTheDocument()
    expect(screen.getAllByRole('alert')).toHaveLength(1)
    expect(screen.getByRole('link', { name: 'prod' })).toHaveAttribute('href', '/servers/prod?tab=monitoring')
    await user.click(screen.getByRole('button', { name: /Dismiss the alert for prod/ }))
    await waitFor(() => expect(screen.queryByText('It may have crashed.')).not.toBeInTheDocument())
  })

  it('stays quiet when the alerts cannot be read', async () => {
    fetchMock.mockResolvedValue(jsonResponse(500, { error: { code: 'internal', message: 'x' } }))
    const { container } = renderApp(<AlertsBanner />)
    await waitFor(() => expect(fetchMock).toHaveBeenCalled())
    expect(container).toBeEmptyDOMElement()
  })
})

describe('niceCeil', () => {
  it('rounds a chart top up to a tidy number', () => {
    expect([0, 7, 23, 140, 2900].map(niceCeil)).toEqual([1, 8, 25, 150, 3000])
  })
})
