import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ServerDetail } from '@/api/types'
import { ConsoleTab } from './ConsoleTab'
import { jsonResponse, renderApp } from '@/test/utils'

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock); vi.stubGlobal('EventSource', undefined) })
afterEach(() => vi.unstubAllGlobals())

const server = (status: ServerDetail['status']) => ({ name: 'prod', mode: 'production', status, console_enabled: true }) as unknown as ServerDetail

describe('ConsoleTab when the server is not running', () => {
  it('starts the server from the notice', async () => {
    const calls: string[] = []
    fetchMock.mockImplementation(async (url: string, init: RequestInit = {}) => {
      calls.push(`${init.method ?? 'GET'} ${url} ${typeof init.body === 'string' ? init.body : ''}`.trim())
      if (url === '/api/v1/servers/prod/control') return jsonResponse(202, { job: 'j1' })
      return jsonResponse(200, { id: 'j1', op: 'control', server: 'prod', state: 'done', events: [], result: null, error: null })
    })
    renderApp(<ConsoleTab server={server('stopped')} />)
    expect(screen.getByText('The server is not running, so there is no live output.')).toBeInTheDocument()
    expect(screen.queryByText(/Overview tab/)).toBeNull()
    await userEvent.click(screen.getByRole('button', { name: 'Start server' }))
    await waitFor(() => expect(calls).toContain('POST /api/v1/servers/prod/control {"action":"start"}'))
  })

  it('offers no Start button when the server folder is missing', () => {
    renderApp(<ConsoleTab server={server('missing')} />)
    expect(screen.getByText('The server is not running, so there is no live output.')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Start server' })).toBeNull()
  })

  it('shows no notice while the server runs', () => {
    renderApp(<ConsoleTab server={server('running')} />)
    expect(screen.queryByText(/not running/)).toBeNull()
  })
})
