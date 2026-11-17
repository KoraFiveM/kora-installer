import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { DeleteFlow } from './DangerTab'
import { FakeEventSource, jsonResponse, renderApp } from '@/test/utils'

const fetchMock = vi.fn()
beforeEach(() => {
  FakeEventSource.instances = []
  fetchMock.mockReset()
  vi.stubGlobal('fetch', fetchMock)
  vi.stubGlobal('EventSource', FakeEventSource)
})
afterEach(() => vi.unstubAllGlobals())

const server = { name: 'kora-dev', managed: true, dir: '/srv/fivem/kora-dev', status: 'stopped' as const }

describe('DeleteFlow', () => {
  it('asks about files, then the database, then needs the server name typed', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
      if (url === '/api/v1/servers/kora-dev' && init?.method === 'DELETE') return jsonResponse(202, { job: 'j9' })
      return jsonResponse(200, { id: 'j9', op: 'server_delete', server: 'kora-dev', state: 'running', events: [], result: null, error: null })
    })
    renderApp(<DeleteFlow server={server} />)
    await user.click(screen.getByRole('button', { name: /delete server/i }))
    expect(await screen.findByText('Keep the server files?')).toBeInTheDocument()
    await user.click(await screen.findByLabelText('Delete the files'))
    await user.click(screen.getByRole('button', { name: 'Next' }))
    expect(screen.getByText('Keep the database?')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Next' }))

    const confirm = screen.getByRole('button', { name: 'Delete server' })
    expect(confirm).toBeDisabled()
    await user.type(screen.getByLabelText(/type kora-dev to confirm/i), 'kora-de')
    expect(confirm).toBeDisabled()
    await user.type(screen.getByLabelText(/type kora-dev to confirm/i), 'v')
    expect(confirm).toBeEnabled()
    await user.click(confirm)

    await waitFor(() => expect(fetchMock.mock.calls.some(([, i]) => i?.method === 'DELETE')).toBe(true))
    const [, init] = fetchMock.mock.calls.find(([, i]) => i?.method === 'DELETE')!
    expect(init.headers['X-Kora']).toBe('1')
    expect(JSON.parse(init.body)).toEqual({ confirm_name: 'kora-dev', keep_files: false, keep_db: true })
  })

  it('asks for the database admin password when dropping the database fails, and retries with it', async () => {
    const user = userEvent.setup()
    let deletes = 0
    fetchMock.mockImplementation(async (url: string, init?: RequestInit) => {
      if (url === '/api/v1/servers/kora-dev' && init?.method === 'DELETE') { deletes += 1; return jsonResponse(202, { job: `j${deletes}` }) }
      if (url === '/api/v1/jobs/j1') {
        return jsonResponse(200, { id: 'j1', op: 'server_delete', server: 'kora-dev', state: 'failed', events: [], result: null, error: { code: 'needs_db_password', message: 'Kora needs the database administrator password.' } })
      }
      return jsonResponse(200, { id: 'j2', op: 'server_delete', server: 'kora-dev', state: 'running', events: [], result: null, error: null })
    })
    renderApp(<DeleteFlow server={server} />)
    await user.click(screen.getByRole('button', { name: /delete server/i }))
    await user.click(await screen.findByLabelText('Delete the files'))
    await user.click(screen.getByRole('button', { name: 'Next' }))
    await user.click(screen.getByLabelText('Delete the database'))
    await user.click(screen.getByRole('button', { name: 'Next' }))
    await user.type(screen.getByLabelText(/type kora-dev to confirm/i), 'kora-dev')
    await user.click(screen.getByRole('button', { name: 'Delete server' }))
    await waitFor(() => expect(FakeEventSource.instances.length).toBeGreaterThan(0))
    act(() => FakeEventSource.instances[0].emit('failed', {}))

    const pw = await screen.findByLabelText(/database administrator password/i, {}, { timeout: 4000 })
    expect(pw).toHaveAttribute('type', 'password')
    const retry = screen.getByRole('button', { name: /try again with this password/i })
    expect(retry).toBeDisabled()
    await user.type(pw, 'hunter2')
    await user.click(retry)
    await waitFor(() => expect(deletes).toBe(2))
    const bodies = fetchMock.mock.calls.filter(([, i]) => i?.method === 'DELETE').map(([, i]) => JSON.parse(i.body))
    expect(bodies[0]).toEqual({ confirm_name: 'kora-dev', keep_files: false, keep_db: false })
    expect(bodies[1]).toEqual({ confirm_name: 'kora-dev', keep_files: false, keep_db: false, db_admin_password: 'hunter2' })
  })

  it('shows only "Remove from list" for an adopted server', async () => {
    const user = userEvent.setup()
    renderApp(<DeleteFlow server={{ ...server, managed: false }} />)
    expect(screen.queryByRole('button', { name: /delete server/i })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Remove from list' }))
    expect(screen.getByText(/Your files stay where they are/)).toBeInTheDocument()
  })

  it('calls the unregister route when confirmed', async () => {
    const user = userEvent.setup()
    fetchMock.mockResolvedValue(new Response(null, { status: 204 }))
    const onDone = vi.fn()
    renderApp(<DeleteFlow server={{ ...server, managed: false }} onDone={onDone} />)
    await user.click(screen.getByRole('button', { name: 'Remove from list' }))
    const buttons = screen.getAllByRole('button', { name: 'Remove from list' })
    await user.click(buttons[buttons.length - 1])
    await waitFor(() => expect(onDone).toHaveBeenCalled())
    expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/servers/kora-dev/registration')
  })
})
