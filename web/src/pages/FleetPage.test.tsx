import { beforeAll, describe, expect, it } from 'vitest'
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import FleetPage from './FleetPage'
import { installMock } from '@/api/mock'
import { renderApp } from '@/test/utils'

beforeAll(() => {
  history.replaceState(null, '', '/')
  installMock()
})

describe('Fleet page', () => {
  it('lists nodes with status and a combined server table, offering only what each node supports', async () => {
    renderApp(<FleetPage />, '/fleet')
    expect(await screen.findByRole('button', { name: 'Remove Garage box' })).toBeInTheDocument()
    expect(screen.getByText('Not reachable')).toBeInTheDocument()
    expect(screen.getByText(/Some nodes are not answering/)).toBeInTheDocument()
    expect(screen.getByText('View only: update it to control it')).toBeInTheDocument()
    // Garage box can be controlled; the old laptop cannot.
    expect(screen.getByRole('button', { name: 'Stop garage-rp on Garage box' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /laptop-dev on Old laptop/ })).not.toBeInTheDocument()
    expect(screen.getAllByText('Update that node to control it from here').length).toBeGreaterThan(0)
    // The token is never shown anywhere.
    expect(screen.queryByText(/token/i, { selector: 'td' })).not.toBeInTheDocument()
  })

  it('adds a node (warning for an open address) and removes one', async () => {
    const user = userEvent.setup()
    renderApp(<FleetPage />, '/fleet')
    await screen.findByRole('button', { name: 'Remove Garage box' })
    await user.click(screen.getByRole('button', { name: /Add a node/ }))
    const dlg = await screen.findByRole('dialog')
    await user.type(within(dlg).getByLabelText('Name'), 'Far away')
    await user.type(within(dlg).getByLabelText('Address'), 'http://203.0.113.5:47120')
    const tok = within(dlg).getByLabelText('Token')
    expect(tok).toHaveAttribute('type', 'password')
    await user.type(tok, 'secret')
    await user.click(within(dlg).getByRole('button', { name: 'Add node' }))
    // (Alerts are drawn by the app shell, not by this page.) The node is listed, with its warning.
    expect(await screen.findByRole('button', { name: 'Remove Far away' })).toBeInTheDocument()
    expect(await screen.findByText(/plain http:\/\/ over the internet/)).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Remove Old laptop' }))
    await user.click(await screen.findByRole('button', { name: 'Remove node' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Remove Old laptop' })).not.toBeInTheDocument())
  })

  it('shows the reason when a node is rejected', async () => {
    const user = userEvent.setup()
    renderApp(<FleetPage />, '/fleet')
    await screen.findByRole('button', { name: 'Remove Garage box' })
    await user.click(screen.getByRole('button', { name: /Add a node/ }))
    const dlg = await screen.findByRole('dialog')
    await user.type(within(dlg).getByLabelText('Name'), 'Bad')
    await user.type(within(dlg).getByLabelText('Address'), 'ftp://nope')
    await user.type(within(dlg).getByLabelText('Token'), 't')
    await user.click(within(dlg).getByRole('button', { name: 'Add node' }))
    expect(await within(dlg).findByText(/must start with http/)).toBeInTheDocument()
  })
})
