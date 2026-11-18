import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactElement } from 'react'
import type { Meta, ServerDetail } from '@/api/types'
import { jsonResponse } from '@/test/utils'
import { ScheduleTab } from './ScheduleTab'

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

describe('ScheduleTab', () => {
  it('is off with the reason for a remote admin while destructive actions are off', async () => {
    fetchMock.mockResolvedValue(jsonResponse(200, { schedule: null, description: null }))
    renderAs(<ScheduleTab server={server} />, { remote: true, identity: { name: 'adam', provider: 'tailscale', role: 'admin' } })
    const button = screen.getByRole('button', { name: 'Turn schedule off' })
    expect(button).toBeDisabled()
    expect(button).toHaveAttribute('title', expect.stringContaining('kora panel remote-actions on'))
    expect(screen.getByText(/turned off for sign-ins that do not come through Tailscale.*kora panel remote-actions on/)).toBeInTheDocument()
  })

  it('says what was saved', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (_url: string, init?: RequestInit) => {
      if (init?.method === 'PUT') return jsonResponse(200, { schedule: JSON.parse(String(init.body)), description: 'x' })
      if (init?.method === 'DELETE') return new Response(null, { status: 204 })
      return jsonResponse(200, { schedule: null, description: null })
    })
    renderAs(<ScheduleTab server={server} />, { remote: true, allow_remote_destructive: true, identity: { name: 'adam', provider: 'tailscale', role: 'admin' } })
    await waitFor(() => expect(fetchMock).toHaveBeenCalled())
    await user.click(screen.getByLabelText('Every day'))
    const time = screen.getByLabelText('Time (24 hours)')
    await user.clear(time)
    await user.type(time, '05:00')
    await user.click(screen.getByRole('button', { name: 'Save schedule' }))
    expect(await screen.findByText('Saved. The server updates itself every day at 05:00.')).toBeInTheDocument()
  })
})
