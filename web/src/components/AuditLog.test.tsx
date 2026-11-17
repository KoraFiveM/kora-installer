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
      if (u.pathname !== '/api/v1/audit') return jsonResponse(404, {})
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

  it('merges what the administrator service did into the table by time, labelled, and respects the filters', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (url: string) => {
      const u = new URL(url, 'http://x')
      if (u.pathname === '/api/v1/audit/service') {
        return jsonResponse(200, {
          available: true,
          all_accounts: false,
          note: null,
          entries: [
            { at: 1_700_000_050, account: 'jo', what: 'backup job set [live]', result: 'done', detail: null },
            { at: 1_700_000_001, account: 'jo', what: 'run system.service [live]', result: 'failed', detail: 'code 1' },
          ],
        })
      }
      return jsonResponse(200, { entries: [row(3, { op: 'server_delete' }), row(2)], more: false, retention_days: 365 })
    })
    renderAs()
    await waitFor(() => expect(screen.getAllByTestId('audit-row')).toHaveLength(4))
    const trs = screen.getAllByTestId('audit-row')
    // newest first across both sources: the service's job at +50, then the panel's rows at +3 and +2, then the service's failure at +1
    expect(trs[0]).toHaveTextContent('Set up scheduled backups as an operating-system job')
    expect(trs[1]).toHaveTextContent('Deleted a server')
    expect(trs[3]).toHaveTextContent('Set up the server’s background service')
    expect(trs[3]).toHaveTextContent('code 1')
    expect(screen.getByText(/only what it did for your account is shown/)).toBeInTheDocument()
    await user.selectOptions(screen.getByLabelText('Result'), 'failed')
    await waitFor(() => expect(screen.getAllByTestId('audit-row').length).toBeLessThan(4))
  })

  it('says why the administrator service has nothing to show, and the panel rows stay', async () => {
    fetchMock.mockImplementation(async (url: string) => {
      const u = new URL(url, 'http://x')
      if (u.pathname === '/api/v1/audit/service') return jsonResponse(200, { available: false, entries: [], all_accounts: false, note: "Kora's administrator service is not installed on this computer, so nothing has been done with administrator rights." })
      return jsonResponse(200, { entries: [row(1)], more: false, retention_days: 365 })
    })
    renderAs()
    expect(await screen.findByTestId('service-audit-note')).toHaveTextContent('not installed on this computer')
    expect(screen.getAllByTestId('audit-row')).toHaveLength(1)
  })
})
