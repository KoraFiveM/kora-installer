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

const declined = () =>
  new ApiError(403, {
    code: 'admin.declined',
    message: 'Windows did not get permission to change the automatic updates, so nothing that needs administrator rights was changed.',
    actions: [{ kind: 'request_admin' }, { kind: 'show_command', command: ['kora', 'schedule', 'off', 'kora-dev'], elevated: true }],
  })

/** The app's banner messages (the shell draws them; here a plain list). */
function Banners() {
  return <ul>{useAlerts().alerts.map((a) => <li key={a.id}>{a.text}</li>)}</ul>
}

describe('Asking Windows for permission from any page', () => {
  it('labels: "Ask again" once Windows was asked, "Ask for administrator permission" before', () => {
    expect(actionLabel({ kind: 'request_admin' }, { code: 'admin.declined' })).toBe('Ask again')
    expect(actionLabel({ kind: 'request_admin' }, { code: 'admin.timed_out' })).toBe('Ask again')
    expect(actionLabel({ kind: 'request_admin' }, { code: 'admin.remote' })).toBe('Ask for administrator permission')
    expect(actionLabel({ kind: 'request_admin' })).toBe('Ask for administrator permission')
  })

  it('on Windows, "Ask again" repeats what failed, telling Kora the person asked', async () => {
    installMock({ scenario: 'windows', speed: 0 })
    const user = userEvent.setup()
    const fetchSpy = vi.spyOn(window, 'fetch')
    const replay = vi.fn(() => api.schedule.remove('kora-dev'))
    const err = declined()
    err.replay = replay
    renderApp(
      <AdminActions>
        <Banners />
        <ErrorNotice error={err} />
      </AdminActions>,
    )
    const alert = (await screen.findByText('Windows did not get permission')).closest('[role="alert"]') as HTMLElement
    const again = await within(alert).findByRole('button', { name: 'Ask again' })
    expect(within(alert).getByText('Show me the command')).toBeInTheDocument()
    await user.click(again)
    await waitFor(() => expect(replay).toHaveBeenCalledTimes(1))
    const sent = fetchSpy.mock.calls.find(([u, i]) => String(u).includes('/schedule') && i?.method === 'DELETE')
    expect(new Headers(sent?.[1]?.headers).get('X-Kora-Admin')).toBe('ask')
    expect(await screen.findByText('Windows gave permission and Kora made the change.')).toBeInTheDocument()
    // Used once: the next change does not ask by itself.
    await api.schedule.remove('kora-dev')
    const last = fetchSpy.mock.calls.at(-1)
    expect(new Headers(last?.[1]?.headers).get('X-Kora-Admin')).toBeNull()
  })

  it('on Linux there is nothing to ask: the command is shown as before', async () => {
    installMock({ scenario: 'default', speed: 0 })
    const err = new ApiError(403, { code: 'needs_elevation', message: 'Changing the automatic updates needs administrator rights.', command: ['kora', 'schedule', 'off', 'x'], actions: [{ kind: 'request_admin' }, { kind: 'show_command', command: ['kora', 'schedule', 'off', 'x'], elevated: true }] })
    renderApp(<AdminActions><ErrorNotice error={err} /></AdminActions>)
    const alert = await screen.findByRole('alert')
    await waitFor(() => expect(within(alert).getByText(/Run the command below as administrator/)).toBeInTheDocument())
    expect(within(alert).queryByRole('button', { name: /Ask/ })).toBeNull()
    expect(within(alert).queryByText('Show me the command')).toBeNull()
  })
})
