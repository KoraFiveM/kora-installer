import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import userEvent from '@testing-library/user-event'
import InstallWizard from './InstallWizard'
import { installMock } from '@/api/mock'
import { api } from '@/api/client'
import type { InstallRequest, Meta } from '@/api/types'
import { AlertsProvider } from '@/lib/app'
import { REMOTE_ACTIONS_COMMAND } from './install/access'
import { renderApp } from '@/test/utils'

beforeAll(() => {
  history.replaceState(null, '', '/')
  installMock() // the wizard runs against the built-in mock backend
})

/** Render the wizard with the `/meta` answer already known, as it is under the app shell. */
function renderWithMeta(meta: Partial<Meta>) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
  qc.setQueryData(['meta'], { version: '0', api: 1, platform: 'linux', ops: [], node: 'n', ...meta })
  return render(
    <QueryClientProvider client={qc}>
      <AlertsProvider>
        <MemoryRouter initialEntries={['/install']}><InstallWizard /></MemoryRouter>
      </AlertsProvider>
    </QueryClientProvider>,
  )
}

const KEY = 'cfxk_abcdefghij_12345'
const heading = (name: string) => screen.findByRole('heading', { name })
const next = (user: ReturnType<typeof userEvent.setup>) => user.click(screen.getByRole('button', { name: 'Next' }))
const installButton = () => screen.getByRole('button', { name: 'Install the server' })

/** What an install with every suggested answer left alone must send. */
async function defaultBody(): Promise<InstallRequest> {
  const d = await api.installs.defaults()
  return {
    mode: 'production', name: d.name, dir: d.dir, hostname: d.hostname, license_key: KEY, ports: d.ports, txadmin_public: false,
    artifact_url: null, game_build: null, resources: 'all', db: 'install_if_missing', service: true, firewall: true, github_owner: null,
    dry_run: false, db_admin: null, start: true, cfx_identifier: null, app: { desktop_shortcut: true },
  }
}

/** Catch what the wizard sends without really starting an install (the answer never arrives). */
function catchStart() {
  return vi.spyOn(api.installs, 'start').mockImplementation(() => new Promise(() => {}))
}

/** Licence key, name, and on to the last screen: the short path. */
async function quickToReview(user: ReturnType<typeof userEvent.setup>) {
  await user.type(await screen.findByLabelText('Licence key'), KEY)
  await next(user)
  await screen.findByLabelText('Server name')
  await next(user)
  await heading('Check and install')
}

beforeEach(() => {
  installMock() // fresh fake data for every test
})
afterEach(() => vi.restoreAllMocks())

describe('Install wizard: the short path', () => {
  it('installs with only the licence key typed, and sends the suggested answers', async () => {
    const user = userEvent.setup()
    const start = catchStart()
    const want = await defaultBody()
    renderApp(<InstallWizard />, '/install')

    // 1. Licence key: numbered steps and a link to the portal. It is the only thing that must be typed.
    const key = await screen.findByLabelText('Licence key')
    expect(await heading('Your licence key')).toBeInTheDocument()
    expect(within(screen.getByRole('list', { name: 'Steps' })).getAllByRole('listitem').map((li) => li.textContent)).toEqual(['1. Your licence key', '2. Name your server', '3. Check and install'])
    const portal = screen.getByRole('link', { name: /Open the Cfx\.re portal/ })
    expect(portal).toHaveAttribute('href', 'https://portal.cfx.re/servers/registration-keys')
    expect(portal).toHaveAttribute('target', '_blank')
    expect(portal).toHaveAttribute('rel', 'noreferrer')
    expect(screen.getAllByRole('listitem').map((li) => li.textContent)).toEqual(expect.arrayContaining([
      'Open the Cfx.re portal and sign in.',
      'Choose "Server keys", then register a new key.',
      'Copy the key and paste it below.',
    ]))
    expect(key).toHaveAttribute('type', 'password')
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled()
    expect(screen.getByText('Paste your licence key. It is a long string of letters and numbers.')).toBeInTheDocument()
    await user.type(key, KEY)
    await user.click(screen.getByRole('button', { name: 'Show the licence key' }))
    expect(key).toHaveAttribute('type', 'text')
    await next(user)

    // 2. Name: already filled in. The folder follows from it and is shown as plain text.
    expect(await heading('Name your server')).toBeInTheDocument()
    expect(screen.getByLabelText('Server name')).toHaveValue(want.name)
    expect(screen.getByText(want.dir)).toBeInTheDocument()
    expect(screen.queryByLabelText('Server folder')).not.toBeInTheDocument()
    expect(screen.getByText(/This will be a server for players\./)).toBeInTheDocument()
    await next(user)

    // 3. Check and install: every suggested answer, in plain words, each with Change.
    expect(await heading('Check and install')).toBeInTheDocument()
    for (const text of ['Provided (hidden)', 'Production (a server for players)', want.name, want.dir, `Game ${want.ports.game}, txAdmin ${want.ports.txadmin}`, 'Recommended', 'Everything', 'Set up for me', 'Automatic', 'Open the game port', 'Yes', 'Not set. Kora makes a local admin with a generated password.']) {
      expect(screen.getByText(text)).toBeInTheDocument()
    }
    for (const row of ['Licence key', 'Kind of server', 'Name', 'Folder', 'Ports', 'FiveM server build', 'Resources', 'Database', 'Firewall', 'Background service', 'Sign in to txAdmin with your Cfx.re account']) {
      expect(screen.getByRole('button', { name: `Change ${row}` })).toBeInTheDocument()
    }
    expect(screen.queryByText(/cfxk_abcdefghij/)).not.toBeInTheDocument()
    expect(await screen.findByText(/Disk space/)).toBeInTheDocument()
    expect(await screen.findByText(/need administrator rights/)).toBeInTheDocument()
    await waitFor(() => expect(installButton()).toBeEnabled(), { timeout: 3000 })
    await user.click(installButton())
    expect(start).toHaveBeenCalledTimes(1)
    expect(start.mock.calls[0][0]).toEqual(want)
  })

  it('"Change" on a line opens that step and comes back to the last screen', async () => {
    const user = userEvent.setup()
    const start = catchStart()
    const want = await defaultBody()
    renderApp(<InstallWizard />, '/install')
    await quickToReview(user)

    await user.click(screen.getByRole('button', { name: 'Change Database' }))
    expect(await heading('Database')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Install the server' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Next' })).not.toBeInTheDocument()
    await user.click(screen.getByLabelText(/Skip the database/))
    await user.click(screen.getByRole('button', { name: 'Back to the summary' }))
    expect(await heading('Check and install')).toBeInTheDocument()
    expect(screen.getByText('Skip')).toBeInTheDocument()
    expect(screen.getByText('Not needed')).toBeInTheDocument()

    // the optional Cfx.re account is one click away
    await user.click(screen.getByRole('button', { name: 'Change Sign in to txAdmin with your Cfx.re account' }))
    await user.type(await screen.findByLabelText('Cfx.re username'), 'JSixty')
    await user.click(screen.getByRole('button', { name: 'Check' }))
    await screen.findByText(/Found JSixty/)
    await user.click(screen.getByRole('button', { name: 'Back to the summary' }))
    expect(await screen.findByText('JSixty (fivem:4465268)')).toBeInTheDocument()

    // the folder line opens the name step with the folder box showing
    await user.click(screen.getByRole('button', { name: 'Change Folder' }))
    expect(await screen.findByLabelText('Server folder')).toHaveValue(want.dir)
    await user.click(screen.getByRole('button', { name: 'Back to the summary' }))

    await waitFor(() => expect(installButton()).toBeEnabled(), { timeout: 3000 })
    await user.click(installButton())
    expect(start.mock.calls[0][0]).toEqual({ ...want, db: 'skip', cfx_identifier: 'fivem:4465268' })
  })

  it('shows a port that is already in use on the last screen, as an alert beside its line', async () => {
    const user = userEvent.setup()
    const d = await api.installs.defaults()
    const free = d.ports.game + 7
    vi.spyOn(api.installs, 'checkPorts').mockImplementation(async (p) => ({
      game: p.game === d.ports.game
        ? { port: p.game, ok: false, text: `${p.game} is already in use. ${free} is free.`, suggestion: free }
        : { port: p.game, ok: true, text: `${p.game} is free`, suggestion: null },
      txadmin: { port: p.txadmin, ok: true, text: `${p.txadmin} is free`, suggestion: null },
    }))
    renderApp(<InstallWizard />, '/install')
    await quickToReview(user)

    const problem = await screen.findByText('One of the ports is already in use. Pick a free one.')
    expect(problem).toHaveAttribute('role', 'alert')
    const why = screen.getByText('The server cannot be installed yet. Fix the problem marked in red above.')
    expect(why).toHaveAttribute('role', 'alert')
    expect(why).not.toHaveClass('text-muted-foreground')
    expect(installButton()).toBeDisabled()

    // fix it from the same line
    await user.click(screen.getByRole('button', { name: 'Change Ports' }))
    await user.click(await screen.findByRole('button', { name: `Use ${free}` }))
    await user.click(screen.getByRole('button', { name: 'Back to the summary' }))
    expect(await screen.findByText(`Game ${free}, txAdmin ${d.ports.txadmin}`)).toBeInTheDocument()
    await waitFor(() => expect(installButton()).toBeEnabled(), { timeout: 3000 })
    expect(screen.queryByText(/cannot be installed yet/)).not.toBeInTheDocument()
    expect(screen.queryByText('One of the ports is already in use. Pick a free one.')).not.toBeInTheDocument()
  })

  it('says so on the last screen when an answer changed there is left unfinished', async () => {
    const user = userEvent.setup()
    renderApp(<InstallWizard />, '/install')
    await quickToReview(user)
    await user.click(screen.getByRole('button', { name: 'Change Resources' }))
    await user.click(await screen.findByLabelText(/Let me pick/))
    await user.click(screen.getByRole('button', { name: 'Back to the summary' }))
    expect(await screen.findByText('Pick at least one resource, or choose to install everything.')).toHaveAttribute('role', 'alert')
    expect(installButton()).toBeDisabled()
  })

  it('a development server adds the GitHub step, and Next says why it waits', async () => {
    const user = userEvent.setup()
    renderApp(<InstallWizard />, '/install')
    await user.type(await screen.findByLabelText('Licence key'), KEY)
    await next(user)
    await user.click(await screen.findByRole('button', { name: 'Make a development server instead' }))
    expect(screen.getByLabelText(/Production/)).toBeChecked()
    await user.click(screen.getByLabelText(/Development/))
    expect(screen.getByText('3. GitHub')).toBeInTheDocument()
    expect(screen.getByText('4. Check and install')).toBeInTheDocument()
    await next(user)
    expect(await heading('GitHub')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled()
    expect(screen.getByText('Sign in to GitHub to continue.')).toBeInTheDocument()
  })

  it('"Continue where I left off" brings the saved answers to the last screen as they are', async () => {
    const user = userEvent.setup()
    const start = catchStart()
    const d = await api.installs.defaults()
    vi.spyOn(api.installs, 'defaults').mockResolvedValue({
      ...d,
      saved: { mode: 'production', name: 'old-one', dir: '/srv/games/old-one', hostname: 'Old One', ports: { game: 30555, txadmin: 40555 }, db: 'skip', resources: { only: ['kora_core', 'chat'] }, service: false, firewall: false, cfx_identifier: 'fivem:99' },
    })
    renderApp(<InstallWizard />, '/install')
    await user.click(await screen.findByRole('button', { name: 'Continue where I left off' }))
    // the licence key is never kept, so it is asked again
    await user.type(await screen.findByLabelText('Licence key'), KEY)
    await next(user)
    expect(await screen.findByLabelText('Server name')).toHaveValue('old-one')
    expect(screen.getByLabelText('Server folder')).toHaveValue('/srv/games/old-one')
    await next(user)
    expect(await heading('Check and install')).toBeInTheDocument()
    for (const text of ['old-one', '/srv/games/old-one', 'Old One', 'Game 30555, txAdmin 40555', '2 picked, plus what they need', 'Skip', 'Leave closed', 'No', 'fivem:99']) {
      expect(screen.getByText(text)).toBeInTheDocument()
    }
    await waitFor(() => expect(installButton()).toBeEnabled(), { timeout: 3000 })
    await user.click(installButton())
    expect(start.mock.calls[0][0]).toMatchObject({ name: 'old-one', dir: '/srv/games/old-one', hostname: 'Old One', ports: { game: 30555, txadmin: 40555 }, db: 'skip', resources: { only: ['kora_core', 'chat'] }, service: false, firewall: false, cfx_identifier: 'fivem:99', license_key: KEY })
  })
})

describe('Install wizard: every step', () => {
  it('"Choose everything myself" walks every step, and untouched answers send the same as the short path', async () => {
    const user = userEvent.setup()
    const start = catchStart()
    const want = await defaultBody()
    renderApp(<InstallWizard />, '/install')
    await screen.findByLabelText('Licence key')
    await user.click(screen.getByRole('button', { name: 'Choose everything myself' }))

    const seen: string[] = []
    const see = async (title: string) => { seen.push((await heading(title)).textContent ?? '') }
    await see('Kind of server')
    expect(screen.getByLabelText(/Production/)).toBeChecked()
    await next(user)
    await see('Name your server')
    expect(screen.getByLabelText('Server folder')).toHaveValue(want.dir)
    await next(user)
    await see('Ports')
    expect(await screen.findAllByText(/is free/, undefined, { timeout: 3000 })).toHaveLength(2)
    await next(user)
    await see('FiveM server build')
    await next(user)
    await see('Resources')
    await next(user)
    // Database: "set it up for me" is first and already chosen, so a clean machine works
    await see('Database')
    const dbChoices = await screen.findAllByRole('radio', { name: /database/i })
    expect(dbChoices[0]).toHaveAccessibleName(/Set up the database for me \(recommended\)/)
    expect(dbChoices[0]).toBeChecked()
    expect(screen.getByLabelText(/Use the database already on this computer/)).not.toBeChecked()
    await next(user)
    await see('Options')
    await next(user)
    await see('Your licence key')
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled()
    await user.type(screen.getByLabelText('Licence key'), KEY)
    await next(user)
    // Cfx.re account: empty means skip
    await see('Your Cfx.re account')
    await next(user)
    await see('Check and install')
    expect(seen).toHaveLength(10)
    expect(within(screen.getByRole('list', { name: 'Steps' })).getAllByRole('listitem')).toHaveLength(10)
    await waitFor(() => expect(installButton()).toBeEnabled(), { timeout: 3000 })
    await user.click(installButton())
    expect(start.mock.calls[0][0]).toEqual(want)
  })

  it('keeps the folder following the name, and can go back to the short setup', async () => {
    const user = userEvent.setup()
    renderApp(<InstallWizard />, '/install')
    await screen.findByLabelText('Licence key')
    await user.click(screen.getByRole('button', { name: 'Choose everything myself' }))
    await screen.findByLabelText(/Production/)
    await next(user)
    const name = await screen.findByLabelText('Server name')
    await user.clear(name)
    await user.type(name, 'Friday Night RP')
    expect(screen.getByLabelText('Server folder')).toHaveValue('/srv/fivem/friday-night-rp')
    await user.click(screen.getByRole('button', { name: 'Use the short setup' }))
    // answers are kept
    await quickToReview(user)
    expect(screen.getByText('friday-night-rp')).toBeInTheDocument()
    expect(screen.getByText('/srv/fivem/friday-night-rp')).toBeInTheDocument()
  })
})

describe('Install wizard: who may install', () => {
  it('tells a remote admin up front that installing is off, with the command, and asks for nothing', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch')
    renderWithMeta({ remote: true, allow_remote_destructive: false, identity: { name: 'sam', provider: 'local', role: 'admin' } })
    const notice = await screen.findByRole('alert')
    expect(notice).toHaveTextContent('Installing a server is turned off for remote sign-ins. To turn it on, run this on the computer that runs Kora (over SSH is fine):')
    expect(notice).toHaveTextContent('Then reload this page.')
    const cmd = notice.querySelector('code')
    expect(cmd).toHaveTextContent(REMOTE_ACTIONS_COMMAND)
    expect(REMOTE_ACTIONS_COMMAND).toBe('kora panel remote-actions on')
    expect(screen.getByRole('link', { name: 'Back to your servers' })).toHaveAttribute('href', '/')
    expect(screen.queryByRole('button', { name: 'Next' })).not.toBeInTheDocument()
    expect(screen.queryByLabelText('Licence key')).not.toBeInTheDocument()
    await new Promise((r) => setTimeout(r, 50))
    expect(fetchSpy).not.toHaveBeenCalled()
    fetchSpy.mockRestore()
  })

  it('tells someone who is not an administrator why, instead of showing the steps', async () => {
    renderWithMeta({ remote: false, identity: { name: 'kim', provider: 'local', role: 'operator' } })
    expect(await screen.findByRole('alert')).toHaveTextContent('Your account (operator) cannot do this. Ask an administrator for admin access.')
    expect(screen.queryByRole('button', { name: 'Next' })).not.toBeInTheDocument()
  })

  it('shows the steps to a remote admin once remote actions are switched on', async () => {
    renderWithMeta({ remote: true, allow_remote_destructive: true, identity: { name: 'sam', provider: 'local', role: 'admin' } })
    expect(await screen.findByLabelText('Licence key')).toBeInTheDocument()
  })
})
