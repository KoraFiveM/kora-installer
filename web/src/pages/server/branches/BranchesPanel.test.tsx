import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ServerDetail } from '@/api/types'
import { api } from '@/api/client'
import { installMock } from '@/api/mock'
import { forgetJobs } from '@/hooks/useJob'
import { renderApp } from '@/test/utils'
import { BranchesPanel } from './BranchesPanel'

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

const dev = { name: 'kora-dev', mode: 'development', status: 'stopped', running: false, console_enabled: false, game_listening: false, txadmin_up: false, dir: '/srv/fivem/kora-dev', ports: { game: 30121, txadmin: 40121 }, created_unix: 0, badge: null, managed: true } as unknown as ServerDetail

// A dialog on top hides the page from the accessibility tree, so look at the list the way a mouse would.
const row = (id: string) => within(screen.getByRole('list', { name: 'Resources and their branches', hidden: true })).getByText(id, { selector: 'span' }).closest('li') as HTMLElement
/** The list is on screen as soon as the page is; its rows arrive with the answer. */
const loaded = () => screen.findByRole('button', { name: 'Change the branch of kora_core' })
async function signIn() {
  await api.github.login()
  await waitFor(async () => expect(((await (await fetch('/api/v1/github/status')).json()) as { signed_in: boolean }).signed_in).toBe(true))
}

describe('BranchesPanel', () => {
  it('shows where every resource is, in plain words, and says updates leave a switched one alone', async () => {
    renderApp(<BranchesPanel server={dev} />)
    await loaded()
    const list = screen.getByRole('list', { name: 'Resources and their branches' })
    expect(within(list).getAllByText('Normal version (dev)').length).toBeGreaterThan(1)
    expect(within(row('kora_phone')).getByText('On branch fix-contacts')).toBeInTheDocument()
    expect(within(row('kora_jobs')).getByText('2 changed files')).toBeInTheDocument()
    expect(screen.getByText(/1 resource is not on the normal version/)).toBeInTheDocument()
    expect(screen.getByText(/Updates leave it alone/)).toBeInTheDocument()
  })

  it('switches a clean resource to a branch from the fork, tells the person to restart, and updates the list', async () => {
    const user = userEvent.setup()
    renderApp(<BranchesPanel server={dev} />)
    await loaded()
    await user.click(screen.getByRole('button', { name: 'Change the branch of kora_core' }))
    const dialog = await screen.findByRole('dialog')
    const select = await within(dialog).findByLabelText('Branch')
    await waitFor(() => expect(within(select).getByRole('option', { name: 'feature/new-ui' })).toBeInTheDocument())
    await user.selectOptions(select, 'origin:feature/new-ui')
    await user.click(within(dialog).getByRole('button', { name: 'Switch' }))
    expect(await within(dialog).findByText(/Now on branch feature\/new-ui/)).toBeInTheDocument()
    expect(within(dialog).getByText(/The server is not running, so it will use the new version when it starts/)).toBeInTheDocument()
    await waitFor(() => expect(within(row('kora_core')).getByText('On branch feature/new-ui')).toBeInTheDocument())
    // Back to the normal version.
    await user.click(within(dialog).getByRole('button', { name: 'Back to the normal version' }))
    expect(await within(dialog).findByText(/Now on the normal version \(dev\)/)).toBeInTheDocument()
  })

  it('never loses changes: it asks, keeps them aside on request, and can bring them back', async () => {
    const user = userEvent.setup()
    renderApp(<BranchesPanel server={dev} />)
    await loaded()
    await user.click(screen.getByRole('button', { name: 'Change the branch of kora_jobs' }))
    const dialog = await screen.findByRole('dialog', { name: /Branch of kora_jobs/ })
    const pick = await within(dialog).findByLabelText('Branch')
    await waitFor(() => expect(within(pick).getByRole('option', { name: 'feature/new-ui' })).toBeInTheDocument())
    await user.selectOptions(pick, 'origin:feature/new-ui')
    await user.click(within(dialog).getByRole('button', { name: 'Switch' }))
    // Nothing happened yet: the person is asked, with the files listed.
    const ask = await screen.findByRole('dialog', { name: 'You have changes that are not saved' })
    expect(within(ask).getByText('config.lua')).toBeInTheDocument()
    expect(within(ask).getByText('client/main.lua')).toBeInTheDocument()
    expect(within(row('kora_jobs')).getByText('2 changed files')).toBeInTheDocument()
    await user.click(within(ask).getByRole('button', { name: 'Keep my changes aside' }))
    expect(await within(dialog).findByText(/Your changes were kept aside/)).toBeInTheDocument()
    const kept = await within(dialog).findByRole('button', { name: /Bring back kept-aside changes 1/ })
    await user.click(kept)
    expect(await within(dialog).findByText(/Your changes are back/)).toBeInTheDocument()
  })

  it('throwing changes away is asked twice, with a warning', async () => {
    const user = userEvent.setup()
    renderApp(<BranchesPanel server={dev} />)
    await loaded()
    await user.click(screen.getByRole('button', { name: 'Change the branch of kora_jobs' }))
    const dialog = await screen.findByRole('dialog', { name: /Branch of kora_jobs/ })
    const pick = await within(dialog).findByLabelText('Branch')
    await waitFor(() => expect(within(pick).getByRole('option', { name: 'feature/new-ui' })).toBeInTheDocument())
    await user.selectOptions(pick, 'origin:feature/new-ui')
    await user.click(within(dialog).getByRole('button', { name: 'Switch' }))
    const ask = await screen.findByRole('dialog', { name: 'You have changes that are not saved' })
    await user.click(within(ask).getByRole('button', { name: 'Throw my changes away…' }))
    expect(within(ask).getByText(/This cannot be undone/)).toBeInTheDocument()
    // Going back does nothing.
    await user.click(within(ask).getByRole('button', { name: 'Go back' }))
    expect(within(row('kora_jobs')).getByText('2 changed files')).toBeInTheDocument()
    await user.click(within(ask).getByRole('button', { name: 'Throw my changes away…' }))
    await user.click(within(ask).getByRole('button', { name: 'Throw my changes away' }))
    await waitFor(() => expect(within(dialog).getByText(/Now on branch feature\/new-ui/)).toBeInTheDocument())
    await waitFor(() => expect(within(row('kora_jobs')).queryByText('2 changed files')).not.toBeInTheDocument())
  })

  it('lists pull requests from both places, labelled, and tries one', async () => {
    const user = userEvent.setup()
    renderApp(<BranchesPanel server={dev} />)
    await loaded()
    await user.click(screen.getByRole('button', { name: 'Change the branch of kora_core' }))
    const dialog = await screen.findByRole('dialog', { name: /Branch of kora_core/ })
    // A pull request is someone else's code: said before anyone tries one.
    expect(within(dialog).getByRole('note')).toHaveTextContent(/someone else's code.*runs on your server.*people you trust/)
    // Not signed in: a plain message, not a git error.
    await user.click(within(dialog).getByRole('button', { name: 'Show open pull requests' }))
    expect(await within(dialog).findByText('You are not signed in to GitHub.')).toBeInTheDocument()
    await signIn()
    await user.click(within(dialog).getByRole('button', { name: 'Try again' }))
    const prs = await within(dialog).findByRole('list', { name: 'Open pull requests' })
    expect(within(prs).getAllByText('Original repository')).toHaveLength(2)
    expect(within(prs).getByText('Your fork')).toBeInTheDocument()
    expect(within(prs).getByText('Draft')).toBeInTheDocument()
    expect(within(prs).getByText('by jo-dev')).toBeInTheDocument()
    await user.click(within(prs).getByRole('button', { name: 'Try pull request #41' }))
    expect(await within(dialog).findByText(/Now on pull request #41/)).toBeInTheDocument()
    await waitFor(() => expect(within(row('kora_core')).getByText('Trying pull request #41')).toBeInTheDocument())
    expect(within(dialog).getByRole('button', { name: 'Back to the normal version' })).toBeInTheDocument()
  })

  it('explains a merged pull request, an unknown one and nonsense, without leaving the dialog', async () => {
    const user = userEvent.setup()
    renderApp(<BranchesPanel server={dev} />)
    await loaded()
    await signIn()
    await user.click(screen.getByRole('button', { name: 'Change the branch of kora_core' }))
    const dialog = await screen.findByRole('dialog', { name: /Branch of kora_core/ })
    const box = within(dialog).getByLabelText('Pull request number or address')
    await user.type(box, '12')
    await user.click(within(dialog).getByRole('button', { name: /^Try it$/ }))
    expect(await within(dialog).findByText(/Pull request #12 has already been merged/)).toBeInTheDocument()
    await user.clear(box)
    await user.type(box, '999')
    await user.click(within(dialog).getByRole('button', { name: /^Try it$/ }))
    expect(await within(dialog).findByText(/There is no pull request #999/)).toBeInTheDocument()
    await user.clear(box)
    await user.type(box, 'rm -rf /')
    await user.click(within(dialog).getByRole('button', { name: /^Try it$/ }))
    expect(await within(dialog).findByText(/doesn't look like a pull request/)).toBeInTheDocument()
    await user.clear(box)
    await user.type(box, 'https://github.com/someone/else/pull/3')
    await user.click(within(dialog).getByRole('button', { name: /^Try it$/ }))
    expect(await within(dialog).findByText(/not this resource's repository or your fork of it/)).toBeInTheDocument()
  })

  it('switches many at once: shows who has the branch first, then switches only those, and asks about changes', async () => {
    const user = userEvent.setup()
    renderApp(<BranchesPanel server={dev} />)
    await loaded()
    await user.type(screen.getByLabelText('Branch name'), 'feature/new-ui')
    await user.click(screen.getByRole('button', { name: 'Check which resources have it' }))
    const which = await screen.findByRole('list', { name: 'Which resources have the branch' })
    expect(await screen.findByText('3 of 4 resources have feature/new-ui')).toBeInTheDocument()
    expect(within(which).getByText('Does not have it')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /Switch the 3 that have it/ }))
    const done = await screen.findByRole('list', { name: 'What happened to each resource' })
    await waitFor(() => expect(within(done).getAllByText('Switched').length).toBe(2))
    expect(within(done).getByText('Has changes of yours')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /Choose what to do with the 1 that has changes/ }))
    const ask = await screen.findByRole('dialog', { name: 'You have changes that are not saved' })
    await user.click(within(ask).getByRole('button', { name: 'Keep my changes aside' }))
    await waitFor(() => expect(screen.getByText(/Your changes are kept aside/)).toBeInTheDocument())
  })

  it('puts everything back to the normal version after a confirmation', async () => {
    const user = userEvent.setup()
    renderApp(<BranchesPanel server={dev} />)
    await loaded()
    await user.click(screen.getByRole('button', { name: 'Back to the normal version, for all' }))
    const ask = await screen.findByRole('dialog', { name: /Put every resource back/ })
    await user.click(within(ask).getByRole('button', { name: 'Switch them all back' }))
    await waitFor(() => expect(within(row('kora_phone')).getByText('Normal version (dev)')).toBeInTheDocument())
  })

  it('a server on released versions answers too (every server can switch a resource)', async () => {
    const r = await fetch('/api/v1/servers/kora-live/branches')
    expect(r.status).toBe(200)
  })
})

describe('the branch where resources are listed', () => {
  it('shows each resource\'s branch on the Resources tab of a Development server', async () => {
    const { ResourcesTab } = await import('../ResourcesTab')
    renderApp(<ResourcesTab server={dev} />)
    const badge = await screen.findByText('On branch fix-contacts')
    // The badge sits on the same row as the resource's name.
    expect(badge.closest('li')).toHaveTextContent('kora_phone')
    expect(screen.getAllByText('Normal version (dev)').length).toBeGreaterThanOrEqual(3)
  })
})
