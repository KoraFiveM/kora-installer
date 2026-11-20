import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import InstallWizard from '../InstallWizard'
import { installMock } from '@/api/mock'
import { renderApp } from '@/test/utils'

// These tests are about the steps, not the setup checklist (SetupGate.test.tsx): treat Cfx and GitHub as set up.
vi.mock('@/pages/install/setup', () => ({ useSetup: () => ({ cfxReady: true, githubReady: true, blocked: false, settled: true }) }))

// Kora's administrator service: with it the administrator parts of an install just happen (no prompt);
// without it (a per-user install) the choices are install it, continue without, or the command.

const KEY = 'cfxk_abcdefghij_12345'
const heading = (name: string) => screen.findByRole('heading', { name })
const next = (user: ReturnType<typeof userEvent.setup>) => user.click(screen.getByRole('button', { name: 'Next' }))
const installButton = () => screen.getByRole('button', { name: 'Install the server' })

beforeAll(() => history.replaceState(null, '', '/'))
afterEach(() => vi.restoreAllMocks())

async function toReview(user: ReturnType<typeof userEvent.setup>, scenario: 'service' | 'noservice', name: string) {
  installMock({ scenario, speed: 0 })
  renderApp(<InstallWizard />, '/install')
  await user.type(await screen.findByLabelText('Licence key'), KEY)
  await next(user)
  const box = await screen.findByLabelText('Server name')
  await user.clear(box)
  await user.type(box, name)
  await next(user)
  await heading('Check and install')
  await waitFor(() => expect(installButton()).toBeEnabled(), { timeout: 3000 })
}

describe('Install wizard and the administrator service', () => {
  it('with the service: says it does the administrator parts and nothing needs approving', async () => {
    const user = userEvent.setup()
    await toReview(user, 'service', 'my-town')
    const notice = (await screen.findByText("Kora's administrator service will do these parts")).closest('[role="status"]') as HTMLElement
    expect(notice).toHaveTextContent('There is nothing to approve')
    expect(notice).toHaveTextContent('Install the background service')
    expect(screen.queryByText(/Windows will ask/)).toBeNull()
  })

  it('without the service: offers to carry on without the parts, and the install is refused with the choices', async () => {
    const user = userEvent.setup()
    await toReview(user, 'noservice', 'my-town')
    expect(await screen.findByText("Kora's administrator service is not installed")).toBeInTheDocument()
    await user.click(installButton())
    expect(await screen.findByText(/its administrator service is not installed on this computer/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Continue without the administrator parts' })).toBeInTheDocument()
    // no handler can launch the installer here: the command is shown instead
    expect(screen.getByText(/open a terminal as administrator/)).toBeInTheDocument()
    expect(screen.getAllByText(/kora service install/).length).toBeGreaterThan(0)
    expect(screen.queryByRole('button', { name: /Ask for administrator permission/ })).toBeNull()
  })
})
