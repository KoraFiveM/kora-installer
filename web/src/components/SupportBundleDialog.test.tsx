import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { BundleListing, Meta } from '@/api/types'
import { jsonResponse } from '@/test/utils'
import SupportBundleDialog from './SupportBundleDialog'

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock) })
afterEach(() => vi.unstubAllGlobals())

const listing: BundleListing = {
  id: 'b1',
  files: [
    { name: 'about.txt', bytes: 80, redactions: 0, hidden: [] },
    { name: 'servers/prod/server.cfg', bytes: 120, redactions: 2, hidden: ['licence key', 'address with a login'] },
  ],
  omitted: [{ name: 'servers/prod/secrets.cfg', why: 'holds the licence key and the database connection string' }],
  total_bytes: 200,
  folder: '/home/me/.kora/support',
  never_uploaded: 'Kora never uploads this file anywhere. It is saved on this computer, and it leaves only if you send it yourself.',
}

function renderIt(meta?: Partial<Meta>) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  if (meta) qc.setQueryData(['meta'], { version: '0', api: 1, platform: 'linux', ops: [], node: 'n', ...meta })
  return render(<QueryClientProvider client={qc}><SupportBundleDialog onClose={() => {}} /></QueryClientProvider>)
}

describe('SupportBundleDialog', () => {
  it('shows what is in the bundle and any file before saving, says it is never uploaded, and saves what was shown', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
      const u = String(url)
      if (u.endsWith('/support/bundle/preview')) return jsonResponse(200, listing)
      if (u.endsWith('/support/bundle/file')) return jsonResponse(200, { name: JSON.parse(String(init?.body)).name, text: 'sv_licenseKey "<hidden: licence key>"' })
      if (u.endsWith('/support/bundle')) return jsonResponse(200, { file_name: 'kora-support-1.zip', path: '/home/me/.kora/support/kora-support-1.zip', bytes: 900, folder: '/home/me/.kora/support' })
      return jsonResponse(200, {})
    })
    renderIt()
    expect(screen.getByText(/Kora never uploads this file anywhere/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /Prepare it and show me what is in it/ }))
    const list = await screen.findByRole('list', { name: 'Files in the support bundle' })
    expect(within(list).getByText('servers/prod/server.cfg')).toBeInTheDocument()
    expect(within(list).getByText(/2 hidden: licence key, address with a login/)).toBeInTheDocument()
    expect(screen.getByText('servers/prod/secrets.cfg')).toBeInTheDocument()
    await user.click(within(list).getByRole('button', { name: 'servers/prod/server.cfg' }))
    expect(await screen.findByLabelText('Text of servers/prod/server.cfg')).toHaveTextContent('<hidden: licence key>')
    await user.click(screen.getByRole('button', { name: 'Save this bundle' }))
    expect(await screen.findByText('Saved')).toBeInTheDocument()
    expect(screen.getByLabelText('Where it was saved')).toHaveTextContent('/home/me/.kora/support/kora-support-1.zip')
    const save = fetchMock.mock.calls.find(([u, i]) => String(u).endsWith('/support/bundle') && i?.method === 'POST')!
    expect(JSON.parse(String(save[1].body))).toEqual({ id: 'b1' })
  })

  it('cancel saves nothing', async () => {
    const user = userEvent.setup()
    fetchMock.mockResolvedValue(jsonResponse(200, listing))
    renderIt()
    await user.click(screen.getByRole('button', { name: /Prepare it/ }))
    await screen.findByText(/This is what the file will hold/)
    await user.click(screen.getByRole('button', { name: 'Cancel, save nothing' }))
    expect(fetchMock.mock.calls.some(([u, i]) => String(u).endsWith('/support/bundle') && i?.method === 'POST')).toBe(false)
  })

  it('an operator is told to ask an administrator and cannot prepare one', async () => {
    renderIt({ identity: { name: 'sam', provider: 'token', role: 'operator' }, remote: true })
    expect(screen.getByText(/cannot make a support bundle/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Prepare it/ })).toBeDisabled()
    await waitFor(() => expect(fetchMock).not.toHaveBeenCalled())
  })
})
