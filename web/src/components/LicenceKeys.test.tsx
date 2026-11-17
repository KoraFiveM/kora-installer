import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { installMock } from '@/api/mock'
import { api } from '@/api/client'
import { renderApp } from '@/test/utils'
import { LicenceKeysCard, RememberedCard } from './LicenceKeys'

const MAIN = 'cfxk_MAINKEYMAINKEY12345_1111'
const TEST = 'cfxk_TESTKEYTESTKEY12345_2222'

beforeAll(() => {
  history.replaceState(null, '', '/')
})
beforeEach(() => {
  installMock({ scenario: 'default', speed: 0 }) // no saved keys, nothing remembered
})

/** The page must never contain a whole key, only its name and last four characters. */
const noFullKey = () => expect(document.body.textContent).not.toMatch(/MAINKEYMAINKEY|TESTKEYTESTKEY/)

describe('Settings: licence keys', () => {
  it('starts empty, adds a key by name and shows only its last four characters', async () => {
    const user = userEvent.setup()
    renderApp(<LicenceKeysCard />)
    expect(await screen.findByText(/No keys saved yet/)).toBeInTheDocument()
    const save = screen.getByRole('button', { name: 'Save key' })
    expect(save).toBeDisabled()
    await user.type(screen.getByLabelText('Name'), 'Main')
    await user.type(screen.getByLabelText('Licence key'), MAIN)
    await user.click(save)
    expect(await screen.findByText('Saved the key ending 1111 as "Main".')).toBeInTheDocument()
    const list = await screen.findByRole('list', { name: 'Saved licence keys' })
    expect(within(list).getByText('Main')).toBeInTheDocument()
    expect(within(list).getByText(/key ending 1111/)).toBeInTheDocument()
    expect((screen.getByLabelText('Licence key') as HTMLInputElement).value).toBe('')
    noFullKey()
  })

  it('says why a name or a key is refused, in plain words', async () => {
    const user = userEvent.setup()
    await api.keys.add('Main', MAIN)
    renderApp(<LicenceKeysCard />)
    await screen.findByText(/key ending 1111/)
    await user.type(screen.getByLabelText('Name'), 'main')
    await user.type(screen.getByLabelText('Licence key'), TEST)
    await user.click(screen.getByRole('button', { name: 'Save key' }))
    expect(await screen.findByText('A saved key is already called "Main". Choose another name.')).toBeInTheDocument()
    await user.clear(screen.getByLabelText('Name'))
    await user.type(screen.getByLabelText('Name'), 'Other')
    await user.clear(screen.getByLabelText('Licence key'))
    await user.type(screen.getByLabelText('Licence key'), MAIN)
    await user.click(screen.getByRole('button', { name: 'Save key' }))
    expect(await screen.findByText(/already saved as "Main"/)).toBeInTheDocument()
    await user.clear(screen.getByLabelText('Licence key'))
    await user.type(screen.getByLabelText('Licence key'), 'not a key')
    await user.click(screen.getByRole('button', { name: 'Save key' }))
    expect(await screen.findByText(/does not look like a Cfx.re licence key/)).toBeInTheDocument()
    noFullKey()
  })

  it('renames, replaces and deletes, telling the person what happens to servers', async () => {
    const user = userEvent.setup()
    await api.keys.add('Main', MAIN)
    renderApp(<LicenceKeysCard />)
    await user.click(await screen.findByRole('button', { name: 'Rename Main' }))
    const dlg = await screen.findByRole('dialog')
    await user.clear(within(dlg).getByLabelText('New name'))
    await user.type(within(dlg).getByLabelText('New name'), 'Live')
    await user.click(within(dlg).getByRole('button', { name: 'Rename' }))
    expect(await screen.findByText('Renamed to "Live".')).toBeInTheDocument()

    await user.click(await screen.findByRole('button', { name: 'Replace the key of Live' }))
    const rep = await screen.findByRole('dialog')
    await user.type(within(rep).getByLabelText('New licence key'), TEST)
    await user.click(within(rep).getByRole('button', { name: 'Replace key' }))
    expect(await screen.findByText(/now holds the key ending 2222/)).toBeInTheDocument()
    expect(screen.getByText(/already installed with the old key keep using it/)).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Delete Live' }))
    const del = await screen.findByRole('dialog')
    expect(within(del).getByText(/keep working: they have their own copy/)).toBeInTheDocument()
    await user.click(within(del).getByRole('button', { name: 'Delete key' }))
    await waitFor(() => expect(screen.queryByRole('list', { name: 'Saved licence keys' })).not.toBeInTheDocument())
    expect(await screen.findByText(/No keys saved yet/)).toBeInTheDocument()
    noFullKey()
  })

  it('lets non-admins see the names but not manage them', async () => {
    await api.keys.add('Main', MAIN)
    vi.spyOn(api.keys, 'list').mockResolvedValue({ keys: [{ name: 'Main', last4: '1111', added: 0, servers: [], stopped_installs: 0 }], can_manage: false, path: null })
    renderApp(<LicenceKeysCard />)
    expect(await screen.findByText('Main')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Delete Main' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Save key' })).not.toBeInTheDocument()
    expect(screen.getByText(/Only an administrator can add, rename, replace or delete keys/)).toBeInTheDocument()
    vi.restoreAllMocks()
  })

  it('explains a damaged keys file with the guided message and offers no way to add', async () => {
    installMock({ scenario: 'trouble', speed: 0 })
    renderApp(<LicenceKeysCard />)
    expect(await screen.findByText(/no key was lost/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Save key' })).not.toBeInTheDocument()
  })
})

describe('Settings: remembered answers', () => {
  it('shows what is remembered and forgets it', async () => {
    const user = userEvent.setup()
    renderApp(<RememberedCard />)
    expect(await screen.findByText('Nothing is remembered yet.')).toBeInTheDocument()
    await api.checks.cfx('JSixty') // a found account is remembered
    await api.profile.set({ github_owner: 'sam-rp' })
    renderApp(<RememberedCard />)
    expect((await screen.findAllByText('JSixty')).length).toBeGreaterThan(0)
    await user.click((await screen.findAllByRole('button', { name: 'Forget Cfx.re name' }))[0])
    await waitFor(async () => expect((await api.profile.get()).cfx_username).toBeNull())
    expect((await api.profile.get()).github_owner).toBe('sam-rp')
  })
})
