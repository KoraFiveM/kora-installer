import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import InstallWizard from '../InstallWizard'
import { installMock } from '@/api/mock'
import { api } from '@/api/client'
import { renderApp } from '@/test/utils'

// Windows: the panel asks Windows for permission itself, once per install (mock scenario "windows").

const KEY = 'cfxk_abcdefghij_12345'
const heading = (name: string) => screen.findByRole('heading', { name })
const next = (user: ReturnType<typeof userEvent.setup>) => user.click(screen.getByRole('button', { name: 'Next' }))
const installButton = () => screen.getByRole('button', { name: 'Install the server' })

beforeAll(() => history.replaceState(null, '', '/'))
afterEach(() => vi.restoreAllMocks())

/** Short path to the summary with this server name (Windows scenario). */
async function toReview(user: ReturnType<typeof userEvent.setup>, name: string) {
  installMock({ scenario: 'windows', speed: 0 })
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

describe('Install wizard on Windows: one permission prompt', () => {
  it('says up front that Windows will ask once, and what for', async () => {
    const user = userEvent.setup()
    await toReview(user, 'my-town')
    const notice = (await screen.findByText('Windows will ask for permission once')).closest('[role="status"]') as HTMLElement
    expect(notice).toHaveTextContent('Register the server as a Windows service')
    expect(notice).toHaveTextContent('Open the game port in the firewall')
    expect(notice).toHaveTextContent('choose Yes')
    expect(notice).toHaveTextContent('flashing shield icon')
    expect(screen.queryByText(/tells you the exact command/)).toBeNull()
  })

  it('"Install without these parts" turns them off before anything starts', async () => {
    const user = userEvent.setup()
    await toReview(user, 'my-town')
    await user.click(await screen.findByRole('button', { name: 'Install without these parts' }))
    await waitFor(() => expect(screen.queryByText('Windows will ask for permission once')).toBeNull())
    expect(screen.getByText('Leave closed')).toBeInTheDocument()
    const start = vi.spyOn(api.installs, 'start').mockImplementation(() => new Promise(() => {}))
    await waitFor(() => expect(installButton()).toBeEnabled())
    await user.click(installButton())
    expect(start.mock.calls[0][0]).toMatchObject({ service: false, firewall: false, db: 'use_existing' })
  })

  it('the person watches the permission step, then the install, in the same job', async () => {
    const user = userEvent.setup()
    await toReview(user, 'my-town')
    await user.click(installButton())
    expect(await heading('All done')).toBeInTheDocument()
    expect(screen.getByText('Administrator permission')).toBeInTheDocument()
  })

  it('a No at the prompt is not a dead end: "Ask again" asks again and the install carries on', async () => {
    const user = userEvent.setup()
    const resume = vi.spyOn(api.installs, 'resume')
    await toReview(user, 'decline-town')
    await user.click(installButton())
    await heading('The install stopped')
    const alert = screen.getByRole('alert')
    expect(alert).toHaveTextContent('Windows did not get permission')
    expect(within(alert).getByRole('button', { name: 'Ask again' })).toBeInTheDocument()
    expect(within(alert).getByRole('button', { name: 'Continue without the administrator parts' })).toBeInTheDocument()
    // The command is there, folded away as the last resort.
    expect(within(alert).getByText('Show me the command')).toBeInTheDocument()
    await user.click(within(alert).getByRole('button', { name: 'Ask again' }))
    expect(await heading('All done')).toBeInTheDocument()
    expect(resume.mock.calls[0][2]).toEqual({ askAdmin: true })
  })

  it('"Continue without the administrator parts" carries on as you, keeping what was done', async () => {
    const user = userEvent.setup()
    const resume = vi.spyOn(api.installs, 'resume')
    await toReview(user, 'nobody-town')
    await user.click(installButton())
    await heading('The install stopped')
    const alert = screen.getByRole('alert')
    expect(alert).toHaveTextContent('Nobody answered the Windows permission prompt in time')
    await user.click(within(alert).getByRole('button', { name: 'Continue without the administrator parts' }))
    expect(await heading('All done')).toBeInTheDocument()
    expect(resume.mock.calls[0][2]).toEqual({ withoutAdmin: true })
    expect(screen.getByText(/players outside this computer cannot connect yet/)).toBeInTheDocument()
  })

  it('signed in from another device: Windows only asks when the person says so', async () => {
    const user = userEvent.setup()
    const start = vi.spyOn(api.installs, 'start')
    await toReview(user, 'remote-town')
    await user.click(installButton())
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('You are signed in from another device')
    expect(start.mock.calls[0][1]).toEqual({})
    await user.click(within(alert).getByRole('button', { name: 'Ask for administrator permission' }))
    expect(await heading('All done')).toBeInTheDocument()
    expect(start.mock.calls[1][1]).toEqual({ askAdmin: true })
  })
})
