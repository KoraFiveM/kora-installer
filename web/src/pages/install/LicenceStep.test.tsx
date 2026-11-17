import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import InstallWizard from '../InstallWizard'
import { installMock } from '@/api/mock'
import { api } from '@/api/client'
import { renderApp } from '@/test/utils'
import { buildRequest, initialForm } from './form'

const MAIN = 'cfxk_MAINKEYMAINKEY12345_1111'
const TEST = 'cfxk_TESTKEYTESTKEY12345_2222'
const PASTED = 'cfxk_PASTEDKEYPASTED1234_3333'

beforeAll(() => {
  history.replaceState(null, '', '/')
})
beforeEach(() => {
  installMock({ scenario: 'default', speed: 0 })
})

const catchStart = () => vi.spyOn(api.installs, 'start').mockImplementation(() => new Promise(() => {}))
const next = (user: ReturnType<typeof userEvent.setup>) => user.click(screen.getByRole('button', { name: 'Next' }))

async function toReview(user: ReturnType<typeof userEvent.setup>) {
  await next(user)
  await screen.findByLabelText('Server name')
  await next(user)
  await screen.findByRole('heading', { name: 'Check and install' })
}

describe('Licence key step', () => {
  it('with no saved keys: paste a key, which is saved by default under "Main", with a clear way not to', async () => {
    const user = userEvent.setup()
    const start = catchStart()
    renderApp(<InstallWizard />, '/install')
    await user.type(await screen.findByLabelText('Licence key'), PASTED)
    const save = screen.getByRole('checkbox', { name: /Save this key on this computer/ })
    expect(save).toBeChecked()
    expect(screen.getByLabelText('Name for this key')).toHaveValue('Main')
    await toReview(user)
    expect(screen.getByText(/It will be saved as "Main"/)).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Install the server' })).toBeEnabled(), { timeout: 3000 })
    await user.click(screen.getByRole('button', { name: 'Install the server' }))
    expect(start.mock.calls[0][0]).toMatchObject({ license_key: PASTED, save_license_key_as: 'Main' })
    expect(start.mock.calls[0][0].license_key_name).toBeUndefined()
    start.mockRestore()
  })

  it('turning "Save" off sends the key without a name and says it will not be saved', async () => {
    const user = userEvent.setup()
    const start = catchStart()
    renderApp(<InstallWizard />, '/install')
    await user.type(await screen.findByLabelText('Licence key'), PASTED)
    await user.click(screen.getByRole('checkbox', { name: /Save this key on this computer/ }))
    expect(screen.getByText(/Not saved\. You will paste it again next time/)).toBeInTheDocument()
    expect(screen.queryByLabelText('Name for this key')).not.toBeInTheDocument()
    await toReview(user)
    expect(screen.getByText(/It will not be saved/)).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Install the server' })).toBeEnabled(), { timeout: 3000 })
    await user.click(screen.getByRole('button', { name: 'Install the server' }))
    expect(start.mock.calls[0][0].save_license_key_as).toBeUndefined()
    expect(start.mock.calls[0][0].license_key).toBe(PASTED)
    start.mockRestore()
  })

  it('with saved keys: the first is picked, shows only the last four, and the request carries the name, not a key', async () => {
    const user = userEvent.setup()
    await api.keys.add('Main', MAIN)
    await api.keys.add('Test server', TEST)
    const start = catchStart()
    renderApp(<InstallWizard />, '/install')
    expect(await screen.findByRole('radio', { name: /Main/ })).toBeChecked()
    expect(screen.getByText(/Key ending 1111/)).toBeInTheDocument()
    expect(screen.queryByLabelText('Licence key')).not.toBeInTheDocument()
    expect(document.body.textContent).not.toMatch(/MAINKEYMAINKEY|TESTKEYTESTKEY/)
    await user.click(screen.getByRole('radio', { name: /Test server/ }))
    await toReview(user)
    expect(screen.getByText('Your saved key "Test server"')).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Install the server' })).toBeEnabled(), { timeout: 3000 })
    await user.click(screen.getByRole('button', { name: 'Install the server' }))
    const sent = start.mock.calls[0][0]
    expect(sent.license_key_name).toBe('Test server')
    expect(sent.license_key).toBeNull()
    expect(JSON.stringify(sent)).not.toContain('TESTKEY')
    start.mockRestore()
  })

  it('"Add a new key" next to the saved ones shows the paste form', async () => {
    const user = userEvent.setup()
    await api.keys.add('Main', MAIN)
    renderApp(<InstallWizard />, '/install')
    await user.click(await screen.findByRole('radio', { name: /Add a new key/ }))
    expect(screen.getByLabelText('Licence key')).toBeInTheDocument()
    expect(screen.getByLabelText('Name for this key')).toHaveValue('Key 2')
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled()
  })

  it('a role that cannot manage keys can only pick, and is told who to ask when there are none', async () => {
    vi.spyOn(api.keys, 'list').mockResolvedValue({ keys: [], can_manage: false, path: null })
    renderApp(<InstallWizard />, '/install')
    expect(await screen.findByText(/Only an administrator can add one/)).toBeInTheDocument()
    expect(screen.queryByLabelText('Licence key')).not.toBeInTheDocument()
    vi.restoreAllMocks()
  })

  it('a key file that cannot be read still lets the person paste a key, but not save it', async () => {
    installMock({ scenario: 'trouble', speed: 0 })
    renderApp(<InstallWizard />, '/install')
    expect(await screen.findByText(/no key was lost/)).toBeInTheDocument()
    expect(await screen.findByLabelText('Licence key')).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: /Save this key on this computer/ })).toBeDisabled()
    expect(screen.getByRole('checkbox', { name: /Save this key on this computer/ })).not.toBeChecked()
  })
})

describe('Remembered Cfx.re name and GitHub owner', () => {
  it('are pre-filled, say where they came from, and can still be changed', async () => {
    const user = userEvent.setup()
    await api.checks.cfx('JSixty')
    await api.profile.set({ github_owner: 'sam-rp' })
    const start = catchStart()
    renderApp(<InstallWizard />, '/install')
    await user.type(await screen.findByLabelText('Licence key'), PASTED)
    await toReview(user)
    expect(screen.getByText(/JSixty \(fivem:4465268\)\. Remembered from your last install/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Change Sign in to txAdmin with your Cfx.re account' }))
    expect(screen.getByLabelText('Cfx.re username')).toHaveValue('JSixty')
    expect(screen.getByText(/Remembered from your last install/)).toBeInTheDocument()
    // Editing the name forgets the remembered account until it is checked again.
    await user.type(screen.getByLabelText('Cfx.re username'), 'x')
    expect(screen.queryByText(/Remembered from your last install/)).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Skip: use a generated local admin instead (password saved in the server folder)' }))
    await user.click(screen.getByRole('button', { name: 'Back to the summary' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Install the server' })).toBeEnabled(), { timeout: 3000 })
    await user.click(screen.getByRole('button', { name: 'Install the server' }))
    expect(start.mock.calls[0][0].cfx_identifier).toBeNull()
    start.mockRestore()
  })

  it('sends the remembered account when it is left alone', async () => {
    const user = userEvent.setup()
    await api.checks.cfx('JSixty')
    const start = catchStart()
    renderApp(<InstallWizard />, '/install')
    await user.type(await screen.findByLabelText('Licence key'), PASTED)
    await toReview(user)
    await waitFor(() => expect(screen.getByRole('button', { name: 'Install the server' })).toBeEnabled(), { timeout: 3000 })
    await user.click(screen.getByRole('button', { name: 'Install the server' }))
    expect(start.mock.calls[0][0].cfx_identifier).toBe('fivem:4465268')
    start.mockRestore()
  })
})

describe('A stopped install', () => {
  it('is offered in the wizard (also after a panel restart) and carries on with its saved key', async () => {
    const user = userEvent.setup()
    await api.keys.add('Main', MAIN)
    const d = await api.installs.defaults()
    const form = { ...initialForm(d, (await api.keys.list()).keys), name: 'fail-town' }
    const { job } = await api.installs.start(buildRequest(form))
    await waitFor(async () => expect((await api.jobs.get(job)).state).toBe('failed'))
    renderApp(<InstallWizard />, '/install')
    expect(await screen.findByText(/"fail-town" stopped at the step build.db/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Carry on' }))
    expect(await screen.findByText(/Installing your server|All done/)).toBeInTheDocument()
  })
})
