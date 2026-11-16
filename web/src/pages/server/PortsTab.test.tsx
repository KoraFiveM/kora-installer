import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { PortsForm } from './PortsTab'
import { validatePorts } from '@/lib/utils'
import { jsonResponse, renderApp } from '@/test/utils'

const fetchMock = vi.fn()
beforeEach(() => { fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock) })
afterEach(() => vi.unstubAllGlobals())

describe('validatePorts', () => {
  it('accepts 1024 to 65535', () => {
    expect(validatePorts(1024, 65535)).toBeNull()
    expect(validatePorts('30120', '40120')).toBeNull()
  })
  it('rejects out-of-range, non-numbers and empty values', () => {
    for (const bad of [[1023, 40120], [30120, 65536], ['abc', 40120], ['', 40120], [30120.5, 40120]] as const) expect(validatePorts(bad[0], bad[1])).toMatch(/1024 to 65535/)
  })
  it('rejects equal ports', () => expect(validatePorts(30120, 30120)).toMatch(/must be different/))
})

describe('PortsForm', () => {
  it('is disabled with an explanation while the server runs', () => {
    renderApp(<PortsForm name="a" ports={{ game: 30120, txadmin: 40120 }} running />)
    expect(screen.getByText(/Stop the server before changing/)).toBeInTheDocument()
    expect(screen.getByLabelText('Game port')).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Change ports' })).toBeDisabled()
  })

  it('shows validation problems and blocks the request', async () => {
    const user = userEvent.setup()
    renderApp(<PortsForm name="a" ports={{ game: 30120, txadmin: 40120 }} running={false} />)
    const game = screen.getByLabelText('Game port')
    await user.clear(game)
    await user.type(game, '80')
    expect(screen.getByRole('alert')).toHaveTextContent('1024 to 65535')
    expect(screen.getByRole('button', { name: 'Change ports' })).toBeDisabled()
    await user.clear(game)
    await user.type(game, '40120')
    expect(screen.getByRole('alert')).toHaveTextContent('must be different')
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('sends the new ports and shows the notes the server returns', async () => {
    const user = userEvent.setup()
    fetchMock.mockResolvedValue(jsonResponse(200, { notes: ['server.cfg was updated.'] }))
    renderApp(<PortsForm name="a" ports={{ game: 30120, txadmin: 40120 }} running={false} />)
    const game = screen.getByLabelText('Game port')
    await user.clear(game)
    await user.type(game, '30130')
    await user.click(screen.getByRole('button', { name: 'Change ports' }))
    await waitFor(() => expect(screen.getByText('server.cfg was updated.')).toBeInTheDocument())
    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('/api/v1/servers/a/ports')
    expect(init.method).toBe('PUT')
    expect(JSON.parse(init.body)).toEqual({ game: 30130, txadmin: 40120 })
  })

  it('shows a conflict error as is', async () => {
    const user = userEvent.setup()
    fetchMock.mockResolvedValue(jsonResponse(409, { error: { code: 'conflict', message: 'Port 30121 is already used by "kora-dev". Pick another one.' } }))
    renderApp(<PortsForm name="a" ports={{ game: 30120, txadmin: 40120 }} running={false} />)
    const game = screen.getByLabelText('Game port')
    await user.clear(game)
    await user.type(game, '30121')
    await user.click(screen.getByRole('button', { name: 'Change ports' }))
    expect(await screen.findByText(/already used by "kora-dev"/)).toBeInTheDocument()
  })
})
