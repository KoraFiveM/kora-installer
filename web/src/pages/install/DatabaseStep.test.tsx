import { beforeEach, describe, expect, it } from 'vitest'
import { screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import InstallWizard from '../InstallWizard'
import { installMock } from '@/api/mock'
import { stopMockDatabase } from '@/api/mock/toolchain'
import { ToolchainActions } from '@/components/ToolchainActions'
import { renderApp } from '@/test/utils'

beforeEach(() => {
  history.replaceState(null, '', '/')
  installMock({ scenario: 'default', speed: 0 })
})

describe('Database step: a stopped database', () => {
  it('offers "Start it for me" when the login test fails because the database is stopped, and tests again by itself', async () => {
    stopMockDatabase()
    const user = userEvent.setup()
    renderApp(<ToolchainActions><InstallWizard /></ToolchainActions>, '/install')
    await user.type(await screen.findByLabelText('Licence key'), 'cfxk_abcdefghij_12345')
    await user.click(screen.getByRole('button', { name: 'Next' }))
    await screen.findByLabelText('Server name')
    await user.click(screen.getByRole('button', { name: 'Next' }))
    await user.click(await screen.findByRole('button', { name: 'Change Database' }))
    await user.click(await screen.findByRole('button', { name: 'Test login' }))
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('installed but not running')
    await user.click(within(alert).getByRole('button', { name: 'Start it for me' }))
    // The dialog closes by itself once the database runs, and the test runs again: logged in.
    expect(await screen.findByText('Kora can log in to the database automatically.', undefined, { timeout: 5000 })).toBeInTheDocument()
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })
})
