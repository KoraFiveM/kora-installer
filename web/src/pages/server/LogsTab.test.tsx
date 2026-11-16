import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactElement } from 'react'
import type { Meta, ServerDetail } from '@/api/types'
import { jsonResponse } from '@/test/utils'
import { LogsTab } from './LogsTab'

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

describe('LogsTab', () => {
  it('shows a viewer a notice, not an error, and asks the server for nothing', () => {
    fetchMock.mockResolvedValue(jsonResponse(403, { error: { code: 'forbidden', message: 'no' } }))
    renderAs(<LogsTab server={server} />, { remote: true, identity: { name: 'vera', provider: 'token', role: 'viewer' } })
    expect(screen.getByText('The logs are not available to you')).toBeInTheDocument()
    expect(screen.getByText(/Ask an administrator for operator access/)).toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('lists the log files for an operator', async () => {
    fetchMock.mockImplementation(async (url: string) => (String(url).includes('/logs/') ? jsonResponse(200, { lines: ['hello'] }) : jsonResponse(200, { files: [{ name: 'server.log', size: 10 }] })))
    renderAs(<LogsTab server={server} />, { remote: true, identity: { name: 'olga', provider: 'token', role: 'operator' } })
    expect(await screen.findByText('hello')).toBeInTheDocument()
  })
})
