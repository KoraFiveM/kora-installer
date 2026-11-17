import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import type { Meta, SystemInfo } from '@/api/types'
import { jsonResponse } from '@/test/utils'
import { CopyConnectButton, OpenFolderButton, TxAdminButton } from './ServerLinks'

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock) })
afterEach(() => vi.unstubAllGlobals())

const sys = (over: Partial<SystemInfo>): SystemInfo => ({ admin: false, elevated: false, platform: 'linux', headless: false, version: '0', local: true, kora_home: '/home/sam/.kora', can_restart: true, ...over })

function setup(system: SystemInfo, meta: Partial<Meta> = {}, ui = <OpenFolderButton name="prod" dir="/srv/fivem/prod" />) {
  fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
    if (url === '/api/v1/system') return jsonResponse(200, system)
    if (url === '/api/v1/servers/prod/open-folder' && init?.method === 'POST') return jsonResponse(204)
    return jsonResponse(200, {})
  })
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
  qc.setQueryData(['meta'], { version: '0', api: 1, platform: 'linux', ops: [], node: 'n', identity: { name: 'sam', provider: 'local', role: 'admin' }, ...meta })
  return render(<QueryClientProvider client={qc}><MemoryRouter>{ui}</MemoryRouter></QueryClientProvider>)
}

describe('OpenFolderButton', () => {
  it('opens the folder when the person is at the computer that runs Kora', async () => {
    setup(sys({ local: true }))
    const b = await screen.findByRole('button', { name: /open folder/i })
    await userEvent.setup().click(b)
    expect(fetchMock).toHaveBeenCalledWith('/api/v1/servers/prod/open-folder', expect.objectContaining({ method: 'POST' }))
    expect(screen.queryByRole('button', { name: /copy path/i })).not.toBeInTheDocument()
  })

  it.each([
    ['a remote session', sys({ local: true }), { remote: true }],
    ['a tunnel to a computer with no desktop', sys({ local: false, headless: true }), {}],
    ['another computer on the network', sys({ local: false }), {}],
  ])('offers Copy path instead for %s', async (_n, system, meta) => {
    setup(system, meta)
    expect(await screen.findByRole('button', { name: /copy path/i })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /open folder/i })).not.toBeInTheDocument()
  })

  it('copies the install path', async () => {
    setup(sys({ local: false }))
    await userEvent.setup().click(await screen.findByRole('button', { name: /copy path/i }))
    expect(await navigator.clipboard.readText()).toBe('/srv/fivem/prod')
  })

  it('falls back to Copy path with the reason when opening fails', async () => {
    setup(sys({ local: true }))
    fetchMock.mockImplementation(async (url: string) =>
      url === '/api/v1/servers/prod/open-folder' ? jsonResponse(400, { error: { code: 'bad_request', message: 'This computer has no desktop.' } }) : jsonResponse(200, sys({ local: true })),
    )
    await userEvent.setup().click(await screen.findByRole('button', { name: /open folder/i }))
    expect(await screen.findByRole('button', { name: /copy path/i })).toBeInTheDocument()
    expect(screen.getByText(/This computer has no desktop/)).toBeInTheDocument()
  })
})

describe('TxAdminButton', () => {
  it('is a link to txAdmin that opens in a new tab when the server is running', async () => {
    setup(sys({ local: true }), {}, <TxAdminButton port={40120} up />)
    const a = await screen.findByRole('link', { name: /open txadmin/i })
    expect(a).toHaveAttribute('href', 'http://localhost:40120')
    expect(a).toHaveAttribute('target', '_blank')
  })

  it('is greyed out, with the reason, while the server is stopped', () => {
    setup(sys({ local: true }), {}, <TxAdminButton port={40120} up={false} />)
    const b = screen.getByRole('button', { name: /open txadmin/i })
    expect(b).toBeDisabled()
    expect(b).toHaveAttribute('title', expect.stringContaining('once the server is running'))
  })
})

describe('CopyConnectButton', () => {
  it('copies the address', async () => {
    setup(sys({}), {}, <CopyConnectButton connect="192.168.1.20:30120" />)
    await userEvent.setup().click(screen.getByRole('button', { name: /copy connect address/i }))
    expect(await navigator.clipboard.readText()).toBe('192.168.1.20:30120')
  })

  it('shows nothing when Kora could not work the address out', () => {
    setup(sys({}), {}, <CopyConnectButton connect={null} />)
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })
})
