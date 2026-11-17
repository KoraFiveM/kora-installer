import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
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
  it('is a link to txAdmin that opens in a new tab on the computer that runs Kora', async () => {
    setup(sys({ local: true }), {}, <TxAdminButton name="prod" port={40120} up />)
    const a = await screen.findByRole('link', { name: /open txadmin/i })
    expect(a).toHaveAttribute('href', 'http://localhost:40120')
    expect(a).toHaveAttribute('target', '_blank')
  })

  it('is greyed out, with the reason, while the server is stopped', () => {
    setup(sys({ local: true }), {}, <TxAdminButton name="prod" port={40120} up={false} />)
    const b = screen.getByRole('button', { name: /open txadmin/i })
    expect(b).toBeDisabled()
    expect(b).toHaveAttribute('title', expect.stringContaining('once the server is running'))
  })

  describe('from another computer', () => {
    const popup = { opener: {} as unknown, location: { href: '' }, close: vi.fn() }
    beforeEach(() => {
      popup.opener = {}
      popup.location.href = ''
      popup.close.mockReset()
    })
    afterEach(() => vi.restoreAllMocks())

    function remoteSetup(ticket: (init?: RequestInit) => Response | Promise<Response>, meta: Partial<Meta> = {}) {
      fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
        if (url === '/api/v1/system') return jsonResponse(200, sys({ local: false }))
        if (url === '/api/v1/servers/prod/txadmin/ticket') return ticket(init)
        return jsonResponse(200, {})
      })
      const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
      qc.setQueryData(['meta'], { version: '0', api: 1, platform: 'linux', ops: [], node: 'n', identity: { name: 'sam', provider: 'local', role: 'admin' }, ...meta })
      return render(<QueryClientProvider client={qc}><MemoryRouter><TxAdminButton name="prod" port={40120} up /></MemoryRouter></QueryClientProvider>)
    }

    it('asks the panel for a one-time ticket and opens it in the tab it opened inside the click', async () => {
      const open = vi.spyOn(window, 'open').mockReturnValue(popup as unknown as Window)
      remoteSetup(() => jsonResponse(200, { url: 'http://my-pc:50120/__kora/enter?t=abc', port: 50120, expires_in: 60, username: 'kora-admin' }))
      await userEvent.setup().click(await screen.findByRole('button', { name: /open txadmin/i }))
      await waitFor(() => expect(popup.location.href).toBe('http://my-pc:50120/__kora/enter?t=abc'))
      expect(open).toHaveBeenCalledWith('about:blank', '_blank')
      expect(popup.opener).toBeNull()
      expect(fetchMock).toHaveBeenCalledWith('/api/v1/servers/prod/txadmin/ticket', expect.objectContaining({ method: 'POST' }))
      // It says honestly that txAdmin has its own sign-in, and which account.
      expect(await screen.findByText(/kora-admin/)).toBeInTheDocument()
      expect(screen.getByText(/cannot sign you in/i)).toBeInTheDocument()
    })

    it('offers a link when the browser blocked the new tab', async () => {
      vi.spyOn(window, 'open').mockReturnValue(null)
      remoteSetup(() => jsonResponse(200, { url: 'http://my-pc:50120/__kora/enter?t=abc', port: 50120, expires_in: 60, username: null }))
      await userEvent.setup().click(await screen.findByRole('button', { name: /open txadmin/i }))
      expect(await screen.findByRole('link', { name: /click here to open txadmin/i })).toHaveAttribute('href', 'http://my-pc:50120/__kora/enter?t=abc')
    })

    it('closes the empty tab and says why when the panel refuses', async () => {
      vi.spyOn(window, 'open').mockReturnValue(popup as unknown as Window)
      remoteSetup(() => jsonResponse(409, { error: { code: 'conflict', message: 'Kora could not listen on 127.0.0.1 port 50120 for txAdmin (address in use).' } }))
      await userEvent.setup().click(await screen.findByRole('button', { name: /open txadmin/i }))
      expect(await screen.findByText(/could not listen/)).toBeInTheDocument()
      expect(popup.close).toHaveBeenCalled()
    })

    it('is not shown to people who are not administrators (txAdmin can run code on the server)', async () => {
      remoteSetup(() => jsonResponse(200, {}), { identity: { name: 'olly', provider: 'token', role: 'operator' }, remote: true })
      await waitFor(() => expect(fetchMock).toHaveBeenCalled())
      expect(screen.queryByRole('button', { name: /open txadmin/i })).not.toBeInTheDocument()
    })

    it('says in words, with the command, why a token administrator cannot open it until remote actions are on', async () => {
      remoteSetup(() => jsonResponse(200, {}), { identity: { name: 'sam', provider: 'token', role: 'admin' }, remote: true, allow_remote_destructive: false })
      const b = await screen.findByRole('button', { name: /open txadmin/i })
      expect(b).toBeDisabled()
      expect(screen.getByText(/kora panel remote-actions on/)).toBeInTheDocument()
    })
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
