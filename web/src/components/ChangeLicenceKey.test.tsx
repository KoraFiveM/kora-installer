import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { installMock } from '@/api/mock'
import { api } from '@/api/client'
import type { ServerDetail } from '@/api/types'
import { renderApp } from '@/test/utils'
import { LicenceKeyCard } from './ChangeLicenceKey'

const MAIN = 'cfxk_MAINKEYMAINKEY12345_1111'
const NEW = 'cfxk_NEWKEYNEWKEYNEW12345_7777'

beforeAll(() => {
  history.replaceState(null, '', '/')
})
beforeEach(() => {
  installMock({ scenario: 'default', speed: 0 })
})

async function server(status: 'running' | 'stopped'): Promise<ServerDetail> {
  const list = await api.servers.list()
  const s = list.find((x) => x.status === status) ?? list[0]
  return { ...s, status, running: status === 'running', game_listening: false, txadmin_up: false, status_detail: { running: status === 'running', game_listening: false, txadmin_up: false }, console_enabled: false }
}

describe('Change licence key (server Overview)', () => {
  it('picks a saved key for a stopped server and says no restart is needed', async () => {
    const user = userEvent.setup()
    await api.keys.add('Main', MAIN)
    const s = await server('stopped')
    renderApp(<LicenceKeyCard server={s} busy={false} restart={() => {}} />)
    await user.click(await screen.findByRole('button', { name: 'Change licence key' }))
    const dlg = await screen.findByRole('dialog')
    expect(await within(dlg).findByRole('radio', { name: /Main/ })).toBeChecked()
    expect(within(dlg).getByText(/stopped, so no restart is needed/)).toBeInTheDocument()
    await user.click(within(dlg).getByRole('button', { name: 'Change licence key' }))
    expect(await screen.findByText(/Licence key changed \(now ending 1111\)/)).toBeInTheDocument()
    expect(screen.getByText(/No restart is needed/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Restart the server now' })).not.toBeInTheDocument()
    // The card now says which saved key the server uses.
    expect(await screen.findByText('Main')).toBeInTheDocument()
  })

  it('tells the person a running server needs a restart and offers it', async () => {
    const user = userEvent.setup()
    await api.keys.add('Main', MAIN)
    const s = await server('running')
    const restart = vi.fn()
    renderApp(<LicenceKeyCard server={s} busy={false} restart={restart} />)
    await user.click(await screen.findByRole('button', { name: 'Change licence key' }))
    const dlg = await screen.findByRole('dialog')
    expect(within(dlg).getByText(/only reads its key when it starts/)).toBeInTheDocument()
    await user.click(within(dlg).getByRole('button', { name: 'Change licence key' }))
    await user.click(await screen.findByRole('button', { name: 'Restart the server now' }))
    expect(restart).toHaveBeenCalled()
  })

  it('a new key can be pasted and saved on the way; the answer shows only its last four', async () => {
    const user = userEvent.setup()
    const s = await server('stopped')
    renderApp(<LicenceKeyCard server={s} busy={false} restart={() => {}} />)
    await user.click(await screen.findByRole('button', { name: 'Change licence key' }))
    const dlg = await screen.findByRole('dialog')
    await user.type(await within(dlg).findByLabelText('New licence key'), NEW)
    expect(within(dlg).getByLabelText('Name for this key')).toBeInTheDocument()
    await user.type(within(dlg).getByLabelText('Name for this key'), 'Main')
    await user.click(within(dlg).getByRole('button', { name: 'Change licence key' }))
    expect(await screen.findByText(/now ending 7777/)).toBeInTheDocument()
    expect((await api.keys.list()).keys.map((k) => k.name)).toEqual(['Main'])
    expect(document.body.textContent).not.toContain('NEWKEYNEWKEY')
  })

  it('shows a refused key in plain words and keeps the dialog open', async () => {
    const user = userEvent.setup()
    const s = await server('stopped')
    renderApp(<LicenceKeyCard server={s} busy={false} restart={() => {}} />)
    await user.click(await screen.findByRole('button', { name: 'Change licence key' }))
    const dlg = await screen.findByRole('dialog')
    await user.type(await within(dlg).findByLabelText('New licence key'), 'short key!')
    await user.clear(within(dlg).getByLabelText('Name for this key'))
    await user.click(within(dlg).getByRole('checkbox', { name: /Save this key/ }))
    await user.click(within(dlg).getByRole('button', { name: 'Change licence key' }))
    expect(await within(dlg).findByText(/does not look like a Cfx.re licence key/)).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('dialog')).toBeInTheDocument())
  })
})
