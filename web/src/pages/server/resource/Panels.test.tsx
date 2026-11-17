import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ResourcePage } from '@/api/types'
import { api } from '@/api/client'
import { installMock } from '@/api/mock'
import { forgetJobs } from '@/hooks/useJob'
import { renderApp } from '@/test/utils'
import { ChangesPanel } from './ChangesPanel'
import { CopyPanel } from './CopyPanel'
import { IssuesPanel } from './IssuesPanel'
import { ReleasesPanel } from './ReleasesPanel'

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

const page = (id: string, over: Partial<ResourcePage['server']> = {}, fork: string | null = 'sam-rp/' + id): ResourcePage => ({
  id,
  folder: id,
  repository: `https://github.com/KoraFiveM/${id}.git`,
  default_branch: 'dev',
  third_party: false,
  branch: { id, folder: id, state: 'default', default_branch: 'dev', label: 'the normal version (dev)', changed_count: 0, stashes: 0 },
  repos: { original: `KoraFiveM/${id}`, fork },
  restart: { needed: false },
  server: { name: 'kora-dev', mode: 'development', source: 'fork', versions: 'latest', running: false, console_enabled: false, console_waiting_for_restart: false, ...over },
})

async function signIn() {
  await api.github.login()
  await waitFor(async () => expect(((await (await fetch('/api/v1/github/status')).json()) as { signed_in: boolean }).signed_in).toBe(true))
}

describe('Your changes', () => {
  it('lists the changed files, shows a readable diff, commits with a message', async () => {
    const user = userEvent.setup()
    renderApp(<ChangesPanel page={page('kora_jobs')} />)
    const files = await screen.findByRole('list', { name: 'Changed files' })
    expect(within(files).getByText('config.lua')).toBeInTheDocument()
    expect(within(files).getByText('New')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Show the changes line by line' }))
    expect(screen.getByLabelText('Changes, line by line')).toHaveTextContent('+Config.Pay = 120')
    const save = screen.getByRole('button', { name: /Save the changes/ })
    expect(save).toBeDisabled()
    await user.type(screen.getByLabelText(/Save these changes with a message/), 'Raise the pay')
    await user.click(save)
    expect(await screen.findByText(/Saved 3 files as commit/)).toBeInTheDocument()
    expect(await screen.findByText('Nothing has changed in this resource\'s folder.')).toBeInTheDocument()
    // The new commit leads the recent ones.
    expect(await screen.findByText('Raise the pay')).toBeInTheDocument()
  })

  it('pushing says whose GitHub account it used; no rights is a guided message', async () => {
    const user = userEvent.setup()
    await signIn()
    renderApp(<ChangesPanel page={page('kora_core')} />)
    await user.click(await screen.findByRole('button', { name: /Push to sam-rp\/kora_core/ }))
    expect(await screen.findByText(/Pushed/)).toBeInTheDocument()
    expect(screen.getAllByTestId('acting-as')[0]).toHaveTextContent('sam-rp')
  })

  it('pushing without rights explains what to do', async () => {
    const user = userEvent.setup()
    await signIn()
    renderApp(<ChangesPanel page={page('kora_phone')} />)
    await user.click(await screen.findByRole('button', { name: /Push to sam-rp\/kora_phone/ }))
    expect(await screen.findByText(/not allowed to push to sam-rp\/kora_phone/)).toBeInTheDocument()
  })

  it('a plain clone has nowhere to push: it says so instead of offering the button', async () => {
    renderApp(<ChangesPanel page={page('kora_core', { name: 'kora-live', source: 'clone', mode: 'production', versions: 'released' }, null)} />)
    expect(await screen.findByText(/plain copy of the original, so there is nowhere of yours/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /^Push to/ })).toBeNull()
  })

  it('a new branch is checked before it is made', async () => {
    const user = userEvent.setup()
    renderApp(<ChangesPanel page={page('kora_core')} />)
    await user.type(await screen.findByLabelText('Name for the new branch'), '--exec=x')
    await user.click(screen.getByRole('button', { name: 'Make the branch' }))
    expect(await screen.findByText(/not a branch name Kora can use/)).toBeInTheDocument()
  })
})

describe('Make my own copy', () => {
  it('turns a plain clone into your own copy and says so', async () => {
    const user = userEvent.setup()
    await signIn()
    renderApp(<CopyPanel page={page('kora_core', { name: 'kora-live', source: 'clone', mode: 'production', versions: 'released' }, null)} />)
    expect(await screen.findByText(/plain copy of/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Make my own copy (fork)' }))
    expect(await screen.findByText(/kora_core now has your own copy \(sam-rp\/kora_core\)/)).toBeInTheDocument()
  })

  it('signed out is the usual guided problem', async () => {
    const user = userEvent.setup()
    renderApp(<CopyPanel page={page('kora_core', { name: 'kora-live', source: 'clone', mode: 'production', versions: 'released' }, null)} />)
    await user.click(await screen.findByRole('button', { name: 'Make my own copy (fork)' }))
    expect(await screen.findByText('You are not signed in to GitHub.')).toBeInTheDocument()
  })

  it('names the copy when there already is one', async () => {
    renderApp(<CopyPanel page={page('kora_core')} />)
    expect(await screen.findByRole('link', { name: 'sam-rp/kora_core' })).toBeInTheDocument()
  })
})

describe('Issues and releases', () => {
  it('lists open issues and comments as the signed-in account', async () => {
    const user = userEvent.setup()
    await signIn()
    renderApp(<IssuesPanel page={page('kora_phone')} />)
    expect(await screen.findByText(/The phone app crashes/)).toBeInTheDocument()
    await user.click(screen.getAllByRole('button', { name: 'Comment' })[0])
    await user.type(screen.getByLabelText('Comment on issue #7'), 'Same here')
    await user.click(screen.getByRole('button', { name: 'Send comment' }))
    expect(await screen.findByText('Your comment was added.')).toBeInTheDocument()
  })

  it('on released versions it says which release the server is pinned to and what is newer', async () => {
    await signIn()
    renderApp(<ReleasesPanel page={page('kora_core', { name: 'kora-live', versions: 'released', source: 'clone', mode: 'production' })} />)
    expect(await screen.findByText(/This server is pinned to/)).toHaveTextContent('1 newer release exists')
    const list = screen.getByRole('list', { name: 'Releases' })
    expect(within(list).getByText('This server')).toBeInTheDocument()
    expect(within(list).getByText('Newer')).toBeInTheDocument()
    expect(within(list).getByText(/New contact app/)).toBeInTheDocument()
  })
})
