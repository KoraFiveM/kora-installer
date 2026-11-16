import { beforeAll, describe, expect, it } from 'vitest'
import { screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import InstallWizard from './InstallWizard'
import { installMock } from '@/api/mock'
import { renderApp } from '@/test/utils'

beforeAll(() => {
  history.replaceState(null, '', '/')
  installMock() // the wizard runs against the built-in mock backend
})

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
    // FiveM build, resources, database, options
    await next()
    await next()
    await next()
    await next()

    // Licence
    const key = await screen.findByLabelText('Licence key')
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
  })
})
