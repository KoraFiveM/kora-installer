import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactElement } from 'react'
import type { Meta, ServerDetail } from '@/api/types'
import { jsonResponse } from '@/test/utils'
import { NotificationsTab } from './NotificationsTab'

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock) })
afterEach(() => vi.unstubAllGlobals())

const server = { name: 'dev', mode: 'development', status: 'stopped' } as unknown as ServerDetail

/** Renders with the `/meta` answer the app shell would have loaded (who is signed in, and from where). */
function renderAs(ui: ReactElement, meta?: Partial<Meta>) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  if (meta) qc.setQueryData(['meta'], { version: '0', api: 1, platform: 'linux', ops: [], node: 'n', ...meta })
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>)
}

const hook = { configured: true, set: true, redacted: 'https://discord.com/api/webhooks/***' }

describe('NotificationsTab', () => {
  it('asks before removing the webhook', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (_url: string, init?: RequestInit) => (init?.method === 'DELETE' ? new Response(null, { status: 204 }) : jsonResponse(200, hook)))
    renderAs(<NotificationsTab server={server} />)
    await user.click(await screen.findByRole('button', { name: 'Remove webhook' }))
    expect(await screen.findByText('Remove the webhook?')).toBeInTheDocument()
    expect(screen.getByText(/Alerts stop being sent/)).toBeInTheDocument()
    expect(fetchMock.mock.calls.some(([, i]) => i?.method === 'DELETE')).toBe(false)
    const buttons = screen.getAllByRole('button', { name: 'Remove webhook' })
    await user.click(buttons[buttons.length - 1])
    await waitFor(() => expect(fetchMock.mock.calls.some(([, i]) => i?.method === 'DELETE')).toBe(true))
  })

  it('lets an operator test but not change the webhook', async () => {
    fetchMock.mockResolvedValue(jsonResponse(200, hook))
    renderAs(<NotificationsTab server={server} />, { remote: true, identity: { name: 'olga', provider: 'token', role: 'operator' } })
    expect(await screen.findByRole('button', { name: 'Send a test message' })).toBeEnabled()
    expect(screen.getByRole('button', { name: 'Remove webhook' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Replace webhook' })).toBeDisabled()
    expect(screen.getByText(/Ask an administrator for admin access/)).toBeInTheDocument()
  })
})
