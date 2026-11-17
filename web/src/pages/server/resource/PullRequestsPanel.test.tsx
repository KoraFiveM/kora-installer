import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ResourcePage } from '@/api/types'
import { api } from '@/api/client'
import { installMock } from '@/api/mock'
import { forgetJobs } from '@/hooks/useJob'
import { renderApp } from '@/test/utils'
import { PullRequestsPanel } from './PullRequestsPanel'

const realFetch = window.fetch
const realEs = window.EventSource
beforeEach(() => {
  sessionStorage.clear()
  forgetJobs()
  installMock({ scenario: 'default', speed: 0 })
})
afterEach(() => {
  window.fetch = realFetch
  window.EventSource = realEs
})

const page = (over: Partial<ResourcePage> = {}): ResourcePage => ({
  id: 'kora_core',
  folder: 'kora_core',
  repository: 'https://github.com/KoraFiveM/kora_core.git',
  default_branch: 'dev',
  third_party: false,
  branch: { id: 'kora_core', folder: 'kora_core', state: 'default', default_branch: 'dev', label: 'the normal version (dev)', changed_count: 0, stashes: 0 },
  repos: { original: 'KoraFiveM/kora_core', fork: 'sam-rp/kora_core' },
  restart: { needed: false },
  server: { name: 'kora-dev', mode: 'development', source: 'fork', versions: 'latest', running: false, console_enabled: false, console_waiting_for_restart: false },
  ...over,
})

async function signIn() {
  await api.github.login()
  await waitFor(async () => expect(((await (await fetch('/api/v1/github/status')).json()) as { signed_in: boolean }).signed_in).toBe(true))
}

describe('Pull requests on the resource page', () => {
  it('says plainly when GitHub is not signed in, then lists both places with checks, reviews and age', async () => {
    const user = userEvent.setup()
    renderApp(<PullRequestsPanel page={page()} />)
    expect(await screen.findByText('You are not signed in to GitHub.')).toBeInTheDocument()
    await signIn()
    await user.click(screen.getByRole('button', { name: 'Try again' }))
    const list = await screen.findByRole('list', { name: 'Open pull requests' })
    expect(within(list).getAllByText('Original repository')).toHaveLength(2)
    expect(within(list).getByText('Your own copy')).toBeInTheDocument()
    expect(within(list).getByText('Draft')).toBeInTheDocument()
    // Checks and reviews in words, not only colour.
    expect(within(list).getByText('Checks failing')).toBeInTheDocument()
    expect(within(list).getByText('Checks running')).toBeInTheDocument()
    expect(within(list).getByText('Checks passing')).toBeInTheDocument()
    expect(within(list).getByText('Changes requested')).toBeInTheDocument()
    expect(within(list).getByText('Approved')).toBeInTheDocument()
    expect(within(list).getByRole('link', { name: 'Open pull request #41 on GitHub' })).toHaveAttribute('href', 'https://github.com/KoraFiveM/kora_core/pull/41')
    // Search.
    await user.type(screen.getByRole('searchbox', { name: 'Search pull requests' }), 'payroll')
    expect(within(list).queryByText(/inventory layout/)).toBeNull()
    expect(within(list).getByText(/payroll rounding/)).toBeInTheDocument()
    await user.clear(screen.getByRole('searchbox', { name: 'Search pull requests' }))
    await user.type(screen.getByRole('searchbox', { name: 'Search pull requests' }), 'nothing like this')
    expect(await screen.findByText('No pull request matches your search.')).toBeInTheDocument()
  })

  it('shows the details of one: failing check, conversation, files, and why merging is not possible', async () => {
    const user = userEvent.setup()
    await signIn()
    renderApp(<PullRequestsPanel page={page()} />)
    const list = await screen.findByRole('list', { name: 'Open pull requests' })
    await user.click(within((await within(list).findByText(/payroll rounding/)).closest('li') as HTMLElement).getByRole('button', { name: 'Details' }))
    const detail = await screen.findByLabelText('Pull request #41')
    expect(within(detail).getByText(/tests: failing/)).toBeInTheDocument()
    expect(within(detail).getByText('client/main.lua')).toBeInTheDocument()
    expect(within(detail).getByText(/asked for changes/)).toBeInTheDocument()
    expect(within(detail).getByRole('button', { name: /^Merge/ })).toBeDisabled()
    expect(within(detail).getByText(/does not have it/)).toBeInTheDocument()
    // Whose account acts is said.
    expect(within(detail).getByTestId('acting-as')).toHaveTextContent('sam-rp')
  })

  it('comments, and merges only after a confirmation, as the signed-in account', async () => {
    const user = userEvent.setup()
    await signIn()
    renderApp(<PullRequestsPanel page={page()} />)
    const list = await screen.findByRole('list', { name: 'Open pull requests' })
    await user.click(within((await within(list).findByText(/My experiment/)).closest('li') as HTMLElement).getByRole('button', { name: 'Details' }))
    const detail = await screen.findByLabelText('Pull request #3')
    await user.type(within(detail).getByLabelText('Your comment or review'), 'Tested, works')
    await user.click(within(detail).getByRole('button', { name: 'Comment' }))
    expect(await within(detail).findByText('Your comment was added.')).toBeInTheDocument()
    await user.click(within(detail).getByRole('button', { name: /^Merge/ }))
    const confirm = await screen.findByRole('dialog')
    expect(confirm).toHaveTextContent(/for real, as sam-rp/)
    await user.click(within(confirm).getByRole('button', { name: 'Merge it' }))
    expect(await screen.findByText(/Pull request #3 was merged/)).toBeInTheDocument()
  })

  it('trying one asks nothing when the folder is clean, and shows the one being tried first', async () => {
    const user = userEvent.setup()
    await signIn()
    const { rerender } = renderApp(<PullRequestsPanel page={page()} />)
    void rerender
    const list = await screen.findByRole('list', { name: 'Open pull requests' })
    await user.click(await within(list).findByRole('button', { name: 'Try pull request #41' }))
    expect(await screen.findByText(/Now on pull request #41/)).toBeInTheDocument()
    expect(screen.getByText(/The running server still has the old version/)).toBeInTheDocument()
  })

  it('shows the pull request being tried with how far behind it is, and the two buttons', async () => {
    await signIn()
    const tried = page({ branch: { id: 'kora_core', folder: 'kora_core', state: 'pr', pr: 41, default_branch: 'dev', label: 'pull request #41', changed_count: 0, stashes: 0 } })
    renderApp(<PullRequestsPanel page={tried} />)
    expect(await screen.findByText('You are trying pull request #41')).toBeInTheDocument()
    expect(await screen.findByTestId('trying-status')).toHaveTextContent('GitHub has 2 newer commits')
    expect(screen.getByRole('button', { name: 'Get the latest commits' })).toBeEnabled()
    expect(screen.getByRole('button', { name: 'Stop trying it' })).toBeEnabled()
  })
})
