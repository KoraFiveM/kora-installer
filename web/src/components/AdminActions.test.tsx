import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { installMock } from '@/api/mock'
import { ApiError, api } from '@/api/client'
import { renderApp } from '@/test/utils'
import { AdminActions } from './AdminActions'
import { ErrorNotice } from './ErrorNotice'
import { useAlerts } from '@/lib/app'
import { actionLabel } from '@/lib/problemActions'

beforeAll(() => history.replaceState(null, '', '/'))
afterEach(() => vi.restoreAllMocks())

const missing = () =>
  new ApiError(403, {
    code: 'needs_elevation',
    message: "Kora needs administrator rights to change the automatic updates, and its administrator service is not installed on this computer.",
    actions: [{ kind: 'install_admin_service' }, { kind: 'show_command', command: ['kora', 'schedule', 'off', 'kora-dev'], elevated: true }],
  })

/** The app's banner messages (the shell draws them; here a plain list). */
function Banners() {
  return <ul>{useAlerts().alerts.map((a) => <li key={a.id}>{a.text}</li>)}</ul>
}

describe("Installing Kora's administrator service from any page", () => {
  it('the button says what it does', () => {
    expect(actionLabel({ kind: 'install_admin_service' })).toBe("Install Kora's administrator service")
  })

  it('on Windows the button runs the installer, then repeats what failed (which now works without asking)', async () => {
    installMock({ scenario: 'windows', speed: 0 })
    const user = userEvent.setup()
    const replay = vi.fn(() => api.schedule.remove('kora-dev'))
    const err = missing()
    err.replay = replay
    renderApp(
      <AdminActions>
        <Banners />
        <ErrorNotice error={err} />
      </AdminActions>,
    )
    const alert = (await screen.findByText('This needs administrator rights')).closest('[role="alert"]') as HTMLElement
    const install = await within(alert).findByRole('button', { name: "Install Kora's administrator service" })
    expect(within(alert).getByText('Show me the command')).toBeInTheDocument()
    await user.click(install)
    expect(await screen.findByText(/Windows asks for administrator permission on the computer that runs Kora/)).toBeInTheDocument()
    await waitFor(() => expect(replay).toHaveBeenCalledTimes(1), { timeout: 5000 })
    expect(await screen.findByText("Kora's administrator service is installed. Kora can now do this without asking.")).toBeInTheDocument()
    const system = await api.system()
    expect(system.admin_service?.state).toBe('ready')
  })

  it('on Linux there is no installer to launch: the command is shown', async () => {
    installMock({ scenario: 'noservice', speed: 0 })
    renderApp(<AdminActions><ErrorNotice error={missing()} /></AdminActions>)
    const alert = await screen.findByRole('alert')
    await waitFor(() => expect(within(alert).getByText(/open a terminal as administrator/)).toBeInTheDocument())
    expect(within(alert).queryByRole('button', { name: /Install Kora's administrator service/ })).toBeNull()
    expect(within(alert).getAllByText(/kora service install/).length).toBeGreaterThan(0)
  })
})
