import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useState } from 'react'
import { screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { GithubOwner } from '@/api/types'
import { jsonResponse, renderApp } from '@/test/utils'
import { initialForm, type WizardForm } from './form'
import { GithubStep } from './Steps'

const fetchMock = vi.fn()
let owners: GithubOwner[] = []
let ownersStatus = 200
const defaults = { name: 'x', dir: '/x', hostname: 'x', ports: { game: 30120, txadmin: 40120 }, saved: null, profile: null, resumable: [] } as never

function Harness({ signedIn = true, onForm }: { signedIn?: boolean; onForm: (f: WizardForm) => void }) {
  const [f, setF] = useState(initialForm(defaults))
  onForm(f)
  return <GithubStep f={{ ...f, source: 'fork' }} set={(p) => setF((x) => ({ ...x, ...p }))} advanced={false} defaultsDir="/x" signedIn={signedIn} onSignedIn={() => {}} />
}

beforeEach(() => {
  fetchMock.mockReset()
  fetchMock.mockImplementation(async (url: string) => {
    if (url.endsWith('/github/owners')) return ownersStatus === 200 ? jsonResponse(200, { owners }) : jsonResponse(ownersStatus, { error: { code: 'github.signed_out', message: 'You are not signed in to GitHub.', actions: [{ kind: 'sign_in_github' }] } })
    return jsonResponse(200, { state: 'ready', login: 'sam' })
  })
  vi.stubGlobal('fetch', fetchMock)
  vi.stubGlobal('EventSource', undefined)
  ownersStatus = 200
})
afterEach(() => vi.unstubAllGlobals())

describe('Where the copies go', () => {
  it('personal account shows the login and picks it', async () => {
    owners = [{ login: 'sam', kind: 'user', can_fork: true }, { login: 'my-org', kind: 'org', can_fork: true }] as GithubOwner[]
    const user = userEvent.setup()
    let form!: WizardForm
    renderApp(<Harness onForm={(f) => { form = f }} />)
    await user.click(await screen.findByLabelText('My personal account (sam)'))
    expect(form.githubOwner).toBe('sam')
  })

  it('an organisation is a second question: which one', async () => {
    owners = [{ login: 'sam', kind: 'user', can_fork: true }, { login: 'my-org', kind: 'org', can_fork: true }, { login: 'other-org', kind: 'org', can_fork: true }] as GithubOwner[]
    const user = userEvent.setup()
    let form!: WizardForm
    renderApp(<Harness onForm={(f) => { form = f }} />)
    await user.click(await screen.findByLabelText('An organisation I belong to'))
    expect(form.githubOwner).toBe('')
    await user.selectOptions(await screen.findByLabelText('Which organisation?'), 'other-org')
    expect(form.githubOwner).toBe('other-org')
  })

  it('with no organisations the choice is off and says what to do', async () => {
    owners = [{ login: 'sam', kind: 'user', can_fork: true }] as GithubOwner[]
    renderApp(<Harness onForm={() => {}} />)
    expect(await screen.findByLabelText(/An organisation I belong to/)).toBeDisabled()
    expect(screen.getByText(/do not belong to an organisation/)).toBeInTheDocument()
  })

  it('an organisation you may not create repositories in is explained, not offered', async () => {
    owners = [{ login: 'sam', kind: 'user', can_fork: true }, { login: 'big-org', kind: 'org', can_fork: false, reason: 'Your organisation does not let members create repositories' }] as GithubOwner[]
    renderApp(<Harness onForm={() => {}} />)
    expect(await screen.findByLabelText(/An organisation I belong to/)).toBeDisabled()
    expect(screen.getByText(/you may not create repositories there: Your organisation does not let members create repositories/)).toBeInTheDocument()
  })

  it('with one organisation you may use and one you may not, only the first can be chosen', async () => {
    owners = [{ login: 'sam', kind: 'user', can_fork: true }, { login: 'ok-org', kind: 'org', can_fork: true }, { login: 'big-org', kind: 'org', can_fork: false, reason: 'no' }] as GithubOwner[]
    const user = userEvent.setup()
    renderApp(<Harness onForm={() => {}} />)
    await user.click(await screen.findByLabelText(/An organisation I belong to/))
    const select = await screen.findByLabelText('Which organisation?')
    expect(screen.getByRole('option', { name: /big-org \(you may not create repositories here\)/ })).toBeDisabled()
    expect(within(select).getByRole('option', { name: 'ok-org' })).toBeEnabled()
  })

  it('not signed in: no choice yet, the sign-in is offered', async () => {
    renderApp(<Harness signedIn={false} onForm={() => {}} />)
    expect(screen.queryByLabelText(/personal account/)).toBeNull()
  })

  it('the list failing is a guided error, not an empty choice', async () => {
    ownersStatus = 403
    renderApp(<Harness onForm={() => {}} />)
    expect(await screen.findByText('You are not signed in to GitHub.')).toBeInTheDocument()
  })
})
