import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { Meta } from '@/api/types'
import type { AuditRow } from '@/api/types'
import { opLabel, whoLabel } from '@/api/store'
import { jsonResponse } from '@/test/utils'
import { AuditLogCard } from './AuditLog'

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock) })
afterEach(() => vi.unstubAllGlobals())

function renderAs(meta?: Partial<Meta>) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  qc.setQueryData(['meta'], { version: '0', api: 1, platform: 'linux', ops: [], node: 'n', ...meta })
  return render(<QueryClientProvider client={qc}><AuditLogCard /></QueryClientProvider>)
}

const row = (id: number, over: Partial<AuditRow> = {}): AuditRow => ({ id, at: 1_700_000_000 + id, op: 'server_control', server: 'live', actor_kind: 'remote', actor_name: 'sam@example.com', provider: 'tailscale', role: 'admin', result: 'ok', detail: null, ...over })

describe('AuditLogCard', () => {
  it('lists who did what in plain words and loads older entries on request', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (url: string) => {
      const u = new URL(url, 'http://x')
      if (u.pathname !== '/api/v1/audit') return jsonResponse(404, {})
      if (u.searchParams.get('before') === '2') return jsonResponse(200, { entries: [row(1, { op: 'key_add', server: null, actor_kind: 'cli', actor_name: 'jamie', provider: null, role: null })], more: false, retention_days: 365 })
      return jsonResponse(200, { entries: [row(3, { result: 'refused', op: 'server_delete', detail: 'HTTP 403' }), row(2, { actor_kind: 'local', actor_name: 'demo', provider: 'local', role: 'admin' })], more: true, retention_days: 365 })
    })
    renderAs()
    expect(await screen.findByText('Deleted a server')).toBeInTheDocument()
    expect(within(screen.getByRole('table')).getByText('Not allowed')).toBeInTheDocument()
    expect(screen.getByText('sam@example.com (signed in from another device with tailscale, admin)')).toBeInTheDocument()
    expect(screen.getByText('demo (on this computer)')).toBeInTheDocument()
    expect(screen.getByText(/kept for 365 days/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Show older entries' }))
    expect(await screen.findByText('jamie (a kora command)')).toBeInTheDocument()
    expect(screen.getByText('Saved a licence key')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Show older entries' })).not.toBeInTheDocument()
  })

  it('filters by result and says so when nothing matches', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (url: string) => {
      const u = new URL(url, 'http://x')
      return jsonResponse(200, { entries: u.searchParams.get('result') === 'failed' ? [] : [row(1)], more: false, retention_days: 365 })
    })
    renderAs()
    await screen.findByTestId('audit-row')
    await user.selectOptions(screen.getByLabelText('Result'), 'failed')
    expect(await screen.findByText(/Nothing matches/)).toBeInTheDocument()
    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => String(u).includes('result=failed'))).toBe(true))
  })

  it('is not shown to people below admin, and asks the server nothing', () => {
    renderAs({ identity: { name: 'kim', provider: 'token', role: 'operator' }, remote: true })
    expect(screen.queryByText('Activity log')).not.toBeInTheDocument()
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('has words for known operations and a fallback for new ones', () => {
    expect(opLabel('key_remove')).toBe('Forgot a saved licence key')
    expect(opLabel('brand_new_thing')).toBe('Brand new thing')
    expect(whoLabel({ actor_kind: 'remote', actor_name: null, provider: null, role: null })).toBe('someone (signed in from another device)')
  })
})
