import { beforeAll, describe, expect, it, vi } from 'vitest'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import InstallWizard from '../InstallWizard'
import { installMock } from '@/api/mock'
import { api } from '@/api/client'
import { renderApp } from '@/test/utils'

beforeAll(() => {
  history.replaceState(null, '', '/')
  installMock()
})

/** Short path to the last screen, then "Change" on the Cfx.re line. Returns the way back to the last screen. */
async function toCfxStep(user: ReturnType<typeof userEvent.setup>) {
  renderApp(<InstallWizard />, '/install')
  const next = () => user.click(screen.getByRole('button', { name: 'Next' }))
  await user.type(await screen.findByLabelText('Licence key'), 'cfxk_abcdefghij_12345')
  await next()
  await screen.findByLabelText('Server name')
  await next()
  await user.click(await screen.findByRole('button', { name: 'Change Sign in to txAdmin with your Cfx.re account' }))
  await screen.findByLabelText('Cfx.re username')
  return () => user.click(screen.getByRole('button', { name: 'Back to the summary' }))
}

const UNCHECKED = 'Check the Cfx.re username, or clear the box to skip this step.'
const installButton = () => screen.getByRole('button', { name: 'Install the server' })

describe('Cfx.re account step', () => {
  it('finds an account, clears it on edit, and puts it in the review and the request', async () => {
    const user = userEvent.setup()
    const start = vi.spyOn(api.installs, 'start')
    const back = await toCfxStep(user)
    const input = screen.getByLabelText('Cfx.re username')
    expect(screen.getByRole('button', { name: 'Check' })).toBeDisabled()
    await user.type(input, 'JSixty')
    // typed but not checked: the step says so
    expect(screen.getByText(UNCHECKED)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Check' }))
    expect(await screen.findByText(/Found JSixty \(fivem:4465268\)/)).toBeInTheDocument()
    expect(screen.queryByText(UNCHECKED)).not.toBeInTheDocument()
    // editing clears the result, and the last screen will not install with an unchecked name
    await user.type(input, 'x')
    expect(screen.queryByText(/Found JSixty/)).not.toBeInTheDocument()
    await back()
    expect(await screen.findByText(UNCHECKED)).toHaveAttribute('role', 'alert')
    expect(installButton()).toBeDisabled()
    await user.click(screen.getByRole('button', { name: 'Change Sign in to txAdmin with your Cfx.re account' }))
    await user.type(screen.getByLabelText('Cfx.re username'), '{Backspace}')
    await user.click(screen.getByRole('button', { name: 'Check' }))
    await screen.findByText(/Found JSixty/)
    await back()
    expect(await screen.findByText('JSixty (fivem:4465268)')).toBeInTheDocument()
    await waitFor(() => expect(installButton()).toBeEnabled(), { timeout: 3000 })
    await user.click(installButton())
    expect(start).toHaveBeenCalled()
    expect(start.mock.calls[0]?.[0].cfx_identifier).toBe('fivem:4465268')
    start.mockRestore()
  })

  it('shows the message when the account is not found or the check fails', async () => {
    const user = userEvent.setup()
    await toCfxStep(user)
    const input = screen.getByLabelText('Cfx.re username')
    await user.type(input, 'nobody-here')
    await user.click(screen.getByRole('button', { name: 'Check' }))
    expect(await screen.findByText(/No Cfx\.re account called "nobody-here"/)).toBeInTheDocument()
    expect(screen.getByText(UNCHECKED)).toBeInTheDocument()
    await user.clear(input)
    await user.type(input, 'offline-guy')
    await user.click(screen.getByRole('button', { name: 'Check' }))
    expect(await screen.findByText(/Could not reach forum\.cfx\.re/)).toBeInTheDocument()
    expect(screen.getByText(UNCHECKED)).toBeInTheDocument()
  })

  it('skips with a generated local admin: no identifier is sent', async () => {
    const user = userEvent.setup()
    const start = vi.spyOn(api.installs, 'start')
    const back = await toCfxStep(user)
    await user.type(screen.getByLabelText('Cfx.re username'), 'JSixty')
    await user.click(screen.getByRole('button', { name: 'Check' }))
    await screen.findByText(/Found JSixty/)
    await user.click(screen.getByRole('button', { name: /^Skip: use a generated local admin/ }))
    expect(screen.getByLabelText('Cfx.re username')).toHaveValue('')
    expect(screen.queryByText(/Found JSixty/)).not.toBeInTheDocument()
    await back()
    expect(await screen.findByText('Not set. Kora makes a local admin with a generated password.')).toBeInTheDocument()
    await waitFor(() => expect(installButton()).toBeEnabled(), { timeout: 3000 })
    await user.click(installButton())
    expect(start.mock.calls[0]?.[0].cfx_identifier).toBeNull()
    start.mockRestore()
  })
})
