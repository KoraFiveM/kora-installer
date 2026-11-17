import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import SystemCheckPage from './SystemCheckPage'
import { ToolchainActions } from '@/components/ToolchainActions'
import { ErrorNotice } from '@/components/ErrorNotice'
import { installMock } from '@/api/mock'
import { stopMockDatabase } from '@/api/mock/toolchain'
import { forgetJobs } from '@/hooks/useJob'
import { renderApp } from '@/test/utils'

function page() {
  return renderApp(<ToolchainActions><SystemCheckPage /></ToolchainActions>, '/system')
}

const line = async (label: string) => (await screen.findByRole('heading', { name: label })).closest('li') as HTMLElement

beforeEach(() => {
  forgetJobs()
})
afterEach(() => vi.restoreAllMocks())

describe('System check page', () => {
  it('lists Git, the GitHub CLI, the sign-in, the database and administrator rights in plain words', async () => {
    installMock({ scenario: 'default', speed: 0 })
    page()
    expect(await screen.findByRole('heading', { name: 'System check' })).toBeInTheDocument()
    for (const label of ['Git', 'GitHub CLI', 'GitHub sign-in', 'Database', 'Administrator rights', 'Background service']) {
      expect(await screen.findByRole('heading', { name: label })).toBeInTheDocument()
    }
    const git = await line('Git')
    expect(within(git).getByText(/Installed 2\.45\.1 \(found on this computer\)/)).toBeInTheDocument()
    expect(within(git).getByText('/usr/bin/git')).toBeInTheDocument()
    // GitHub is not signed in (default world): the sign-in line offers the sign-in button, nothing else is wrong.
    const gh = await line('GitHub sign-in')
    expect(within(gh).getByText('Needs fixing')).toBeInTheDocument()
    expect(within(gh).getByRole('button', { name: 'Sign in to GitHub' })).toBeInTheDocument()
    expect(screen.getByText('1 thing needs fixing.')).toBeInTheDocument()
  })

  it('installs the GitHub CLI for the user: a failed download offers Try again, the second try works, and the page updates', async () => {
    installMock({ scenario: 'trouble', speed: 0 })
    const user = userEvent.setup()
    page()
    const ghLine = await line('GitHub CLI')
    expect(within(ghLine).getByText('Not installed.')).toBeInTheDocument()
    // GitHub waits for the CLI: no second, confusing fix on that line.
    const signIn = await line('GitHub sign-in')
    expect(within(signIn).queryByRole('button')).not.toBeInTheDocument()

    await user.click(within(ghLine).getByRole('button', { name: 'Install the GitHub CLI for me' }))
    const dialog = await screen.findByRole('dialog', { name: 'Install the GitHub CLI' })
    expect(within(dialog).getByText(/checked against a known checksum/)).toBeInTheDocument()
    // First download fails (offline): plain words, a way to try again, a manual link.
    expect(await within(dialog).findByText(/Kora could not download the GitHub CLI/)).toBeInTheDocument()
    expect(within(dialog).getByRole('link', { name: /cli\.github\.com/ })).toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: 'Try again' }))
    expect(await within(dialog).findByText('Done')).toBeInTheDocument()
    await user.click(within(dialog).getAllByRole('button', { name: 'Close' })[0])
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    // The line is fixed without reloading, and the sign-in line now asks for the sign-in.
    const after = await line('GitHub CLI')
    await waitFor(() => expect(within(after).getByText(/Installed 2\.104\.0 \(Kora's own copy\)/)).toBeInTheDocument())
    await waitFor(async () => expect(within(await line('GitHub sign-in')).getByRole('button', { name: 'Sign in to GitHub' })).toBeInTheDocument())
  })

  it('starts a stopped database with "Start it for me"', async () => {
    installMock({ scenario: 'default', speed: 0 })
    stopMockDatabase()
    const user = userEvent.setup()
    page()
    const db = await line('Database')
    expect(within(db).getAllByText(/installed but not running/).length).toBeGreaterThan(0)
    await user.click(within(db).getByRole('button', { name: 'Start it for me' }))
    const dialog = await screen.findByRole('dialog', { name: 'Start the database' })
    expect(await within(dialog).findByText('Done')).toBeInTheDocument()
    await user.click(within(dialog).getAllByRole('button', { name: 'Close' })[0])
    await waitFor(async () => expect(within(await line('Database')).getByText('mariadb 11.4.2 is running.')).toBeInTheDocument())
  })

  it('shows the administrator command when starting the database needs rights Kora does not have', async () => {
    installMock({ scenario: 'trouble', speed: 0 })
    const user = userEvent.setup()
    page()
    const db = await line('Database')
    await user.click(within(db).getByRole('button', { name: 'Start it for me' }))
    const dialog = await screen.findByRole('dialog', { name: 'Start the database' })
    expect((await within(dialog).findAllByText(/needs administrator rights/)).length).toBeGreaterThan(0)
    expect(await within(dialog).findByLabelText('Command to run')).toHaveTextContent(/kora check --start-database/)
  })

  it('"Check again" asks again', async () => {
    installMock({ scenario: 'default', speed: 0 })
    const user = userEvent.setup()
    page()
    await screen.findByRole('heading', { name: 'Git' })
    stopMockDatabase()
    await user.click(screen.getByRole('button', { name: 'Check again' }))
    await waitFor(async () => expect(within(await line('Database')).getByRole('button', { name: 'Start it for me' })).toBeInTheDocument())
  })
})

describe('Links to the System check', () => {
  it('errors about tools, GitHub or the database link to it (except on the page itself)', () => {
    installMock({ scenario: 'default', speed: 0 })
    renderApp(<ErrorNotice error={{ code: 'github.gh_missing', message: 'The GitHub CLI (gh) is not installed.', actions: [] }} />, '/install')
    expect(screen.getByRole('link', { name: 'Open the System check' })).toHaveAttribute('href', '/system')
  })

  it('an unrelated error has no such link', () => {
    renderApp(<ErrorNotice error={{ code: 'install.ports_busy', message: 'A port is busy.', actions: [] }} />, '/install')
    expect(screen.queryByRole('link', { name: 'Open the System check' })).not.toBeInTheDocument()
  })
})
