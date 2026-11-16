import { beforeAll, describe, expect, it, vi } from 'vitest'
import { screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import InstallWizard from '../InstallWizard'
import { installMock } from '@/api/mock'
import { api } from '@/api/client'
import { renderApp } from '@/test/utils'

beforeAll(() => {
  history.replaceState(null, '', '/')
  installMock()
})

async function toCfxStep(user: ReturnType<typeof userEvent.setup>) {
  renderApp(<InstallWizard />, '/install')
  const next = () => user.click(screen.getByRole('button', { name: 'Next' }))
  await screen.findByLabelText(/Production/)
  await next()
  await screen.findByLabelText('Server name')
  await next()
  await screen.findAllByText(/is free/, undefined, { timeout: 3000 })
  await next()
  for (let i = 0; i < 4; i++) await next()
  await user.type(await screen.findByLabelText('Licence key'), 'cfxk_abcdefghij_12345')
  await next()
  await screen.findByLabelText('Cfx.re username')
  return next
}

describe('Cfx.re account step', () => {
  it('finds an account, clears it on edit, and puts it in the review and the request', async () => {
    const user = userEvent.setup()
    const start = vi.spyOn(api.installs, 'start')
    const next = await toCfxStep(user)
    const input = screen.getByLabelText('Cfx.re username')
    expect(screen.getByRole('button', { name: 'Check' })).toBeDisabled()
    await user.type(input, 'JSixty')
    // typed but not checked: Next is blocked
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled()
    await user.click(screen.getByRole('button', { name: 'Check' }))
    expect(await screen.findByText(/Found JSixty \(fivem:4465268\)/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Next' })).toBeEnabled()
    // editing clears the result
    await user.type(input, 'x')
    expect(screen.queryByText(/Found JSixty/)).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled()
    await user.type(input, '{Backspace}')
    await user.click(screen.getByRole('button', { name: 'Check' }))
    await screen.findByText(/Found JSixty/)
    await next()
    expect(await screen.findByText('JSixty (fivem:4465268)')).toBeInTheDocument()
    await user.click(await screen.findByRole('button', { name: 'Install the server' }))
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
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled()
    await user.clear(input)
    await user.type(input, 'offline-guy')
    await user.click(screen.getByRole('button', { name: 'Check' }))
    expect(await screen.findByText(/Could not reach forum\.cfx\.re/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled()
  })

  it('skips with a generated local admin: no identifier is sent', async () => {
    const user = userEvent.setup()
    const start = vi.spyOn(api.installs, 'start')
    const next = await toCfxStep(user)
    await user.type(screen.getByLabelText('Cfx.re username'), 'JSixty')
    await user.click(screen.getByRole('button', { name: 'Check' }))
    await screen.findByText(/Found JSixty/)
    await user.click(screen.getByRole('button', { name: /^Skip: use a generated local admin/ }))
    expect(screen.getByLabelText('Cfx.re username')).toHaveValue('')
    expect(screen.queryByText(/Found JSixty/)).not.toBeInTheDocument()
    await next()
    expect(await screen.findByText('A generated local account')).toBeInTheDocument()
    await user.click(await screen.findByRole('button', { name: 'Install the server' }))
    expect(start.mock.calls[0]?.[0].cfx_identifier).toBeNull()
    start.mockRestore()
  })
})
