import { beforeAll, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import userEvent from '@testing-library/user-event'
import InstallWizard from './InstallWizard'
import { installMock } from '@/api/mock'
import type { Meta } from '@/api/types'
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

describe('Install wizard', () => {
  it('walks a production install from Mode to Review without showing secrets', async () => {
    const user = userEvent.setup()
    renderApp(<InstallWizard />, '/install')
    const next = () => user.click(screen.getByRole('button', { name: 'Next' }))

    // Mode
    expect(await screen.findByLabelText(/Production/)).toBeChecked()
    await next()

    // Name and folder: the folder follows the name
    const name = await screen.findByLabelText('Server name')
    await user.clear(name)
    await user.type(name, 'Friday Night RP')
    expect(screen.getByLabelText('Server folder')).toHaveValue('/srv/fivem/friday-night-rp')
    await next()

    // Ports: the suggested ports skip the ones kora-live already uses; both lines report "is free"
    expect(await screen.findAllByText(/is free/, undefined, { timeout: 3000 })).toHaveLength(2)
    await next()
    // FiveM build, resources
    await next()
    await next()
    // Database: "set it up for me" is first and already chosen, so a clean machine works
    const dbChoices = await screen.findAllByRole('radio', { name: /database/i })
    expect(dbChoices[0]).toHaveAccessibleName(/Set up the database for me \(recommended\)/)
    expect(dbChoices[0]).toBeChecked()
    expect(screen.getByLabelText(/Use the database already on this computer/)).not.toBeChecked()
    await next()
    // Options
    await next()

    // Licence: numbered steps and a link to the portal
    const key = await screen.findByLabelText('Licence key')
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
    await user.type(key, 'cfxk_abcdefghij_12345')
    await user.click(screen.getByRole('button', { name: 'Show the licence key' }))
    expect(key).toHaveAttribute('type', 'text')
    await next()

    // Cfx.re account: empty means skip
    expect(await screen.findByRole('heading', { name: 'Your Cfx.re account' })).toBeInTheDocument()
    await next()

    // Review
    expect(await screen.findByRole('heading', { name: 'Review' })).toBeInTheDocument()
    expect(screen.getByText('friday-night-rp')).toBeInTheDocument()
    expect(screen.getByText('A generated local account')).toBeInTheDocument()
    expect(screen.getByText('Provided (hidden)')).toBeInTheDocument()
    expect(screen.queryByText(/cfxk_abcdefghij/)).not.toBeInTheDocument()
    expect(await screen.findByText(/Disk space/)).toBeInTheDocument()
    expect(await screen.findByText(/need administrator rights/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Install the server' })).toBeEnabled()
    expect(screen.getByText('Set up for me')).toBeInTheDocument()
  })

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
    expect(screen.queryByLabelText(/Production/)).not.toBeInTheDocument()
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
    expect(await screen.findByLabelText(/Production/)).toBeChecked()
  })
})
