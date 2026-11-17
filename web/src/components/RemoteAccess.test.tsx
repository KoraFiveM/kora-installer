import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactElement } from 'react'
import type { Meta, RemoteSettings } from '@/api/types'
import { jsonResponse } from '@/test/utils'
import { RemoteAccessCard } from './RemoteAccess'

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock) })
afterEach(() => vi.unstubAllGlobals())

/** Renders with the `/meta` answer the app shell would have loaded (who is signed in, and from where). */
function renderAs(ui: ReactElement, meta?: Partial<Meta>) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  if (meta) qc.setQueryData(['meta'], { version: '0', api: 1, platform: 'linux', ops: [], node: 'n', ...meta })
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>)
}

const settings: RemoteSettings = { enabled: true, enabled_by_flag: false, bind: '127.0.0.1', port: 8600, tailnet_url: null, tailscale: true, tailscale_users: { 'sam@example.com': 'admin', 'kim@example.com': 'viewer' }, allowed_hosts: [], allow_remote_destructive: false, users: [], saved_to: null, require_strong_auth: false }

describe('RemoteAccessCard', () => {
  it('asks before removing a person or a Tailscale account', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
      if (url === '/api/v1/users') return jsonResponse(200, { users: [{ name: 'bob', role: 'operator', created: 0, passkeys: 0, code: false, legacy_token: true }] })
      if (init?.method === 'DELETE') return new Response(null, { status: 204 })
      if (init?.method === 'PUT') return jsonResponse(200, { ...settings, ...JSON.parse(String(init.body)) })
      return jsonResponse(200, settings)
    })
    renderAs(<RemoteAccessCard />)

    await user.click(await screen.findByRole('button', { name: 'Remove sam@example.com' }))
    expect(screen.getByText('Remove sam@example.com?')).toBeInTheDocument()
    expect(screen.getByText(/cannot sign in again until you add them back/)).toBeInTheDocument()
    expect(fetchMock.mock.calls.some(([, i]) => i?.method === 'PUT')).toBe(false)
    await user.click(screen.getByRole('button', { name: 'Remove' }))
    await waitFor(() => expect(fetchMock.mock.calls.some(([, i]) => i?.method === 'PUT')).toBe(true))
    const [, put] = fetchMock.mock.calls.find(([, i]) => i?.method === 'PUT')!
    expect(JSON.parse(put.body)).toEqual({ tailscale_users: { 'kim@example.com': 'viewer' } })
    await waitFor(() => expect(screen.queryByText(/cannot sign in again/)).not.toBeInTheDocument())

    await user.click(await screen.findByRole('button', { name: 'Remove bob' }))
    expect(screen.getByText('Remove bob?')).toBeInTheDocument()
    expect(screen.getByText(/passkeys and codes are deleted/)).toBeInTheDocument()
    expect(fetchMock.mock.calls.some(([, i]) => i?.method === 'DELETE')).toBe(false)
    await user.click(screen.getByRole('button', { name: 'Remove' }))
    await waitFor(() => expect(fetchMock.mock.calls.some(([u, i]) => u === '/api/v1/users/bob' && i?.method === 'DELETE')).toBe(true))
  })
})
