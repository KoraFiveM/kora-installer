import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { api, ApiError, setReauthHandler, setSessionSecret } from '@/api/client'
import type { AuthOptions } from '@/api/types'
import { jsonResponse } from '@/test/utils'
import { Enrol } from './Enrol'
import { RemoteSignIn } from './RemoteSignIn'
import { AccountCard } from './AccountCard'
import { ReauthProvider } from './ReauthProvider'

const fetchMock = vi.fn()
beforeEach(() => {
  fetchMock.mockReset()
  vi.stubGlobal('fetch', fetchMock)
})
afterEach(() => {
  vi.unstubAllGlobals()
  setReauthHandler(null)
  setSessionSecret(null)
})

const body = (i: number) => JSON.parse(fetchMock.mock.calls[i][1].body)
const options: AuthOptions = { passkey: true, passkey_problem: null, code: true, token: true, tailscale: false, strong_required: false }

/** A browser that has passkeys (jsdom does not). */
function withPasskeys() {
  vi.stubGlobal('isSecureContext', true)
  vi.stubGlobal('PublicKeyCredential', function PublicKeyCredential() {})
  const create = vi.fn()
  const get = vi.fn()
  Object.defineProperty(navigator, 'credentials', { value: { create, get }, configurable: true })
  return { create, get }
}

describe('sign-in from another computer', () => {
  it('offers one-time codes when this browser has no passkeys, and signs in', async () => {
    const user = userEvent.setup()
    const done = vi.fn()
    fetchMock.mockResolvedValue(jsonResponse(200, { step: 'signed_in', identity: { name: 'amy', provider: 'code', role: 'operator' }, secret: 's3', notice: null }))
    render(<RemoteSignIn options={options} onSignedIn={done} />)
    expect(screen.getByLabelText('Password')).toBeInTheDocument()
    await user.type(screen.getByLabelText('Your name'), 'amy')
    await user.type(screen.getByLabelText('Password'), 'plum otter lantern quietly')
    await user.type(screen.getByLabelText('One-time code'), '123456')
    await user.click(screen.getByRole('button', { name: 'Sign in with a code' }))
    await waitFor(() => expect(done).toHaveBeenCalled())
    expect(body(0)).toEqual({ step: 'code', user: 'amy', password: 'plum otter lantern quietly', code: '123456' })
    expect(window.localStorage.getItem('kora_session_secret')).toBe('s3')
  })

  it('shows the panel\'s words for a wrong password and clears the secret fields', async () => {
    const user = userEvent.setup()
    fetchMock.mockResolvedValue(jsonResponse(401, { error: { code: 'unauthorized', message: 'That name, password or code is not right (or the account is locked for a few minutes).' } }))
    render(<RemoteSignIn options={options} onSignedIn={() => undefined} />)
    await user.type(screen.getByLabelText('Your name'), 'amy')
    await user.type(screen.getByLabelText('Password'), 'x')
    await user.type(screen.getByLabelText('One-time code'), '1')
    await user.click(screen.getByRole('button', { name: 'Sign in with a code' }))
    expect(await screen.findByText(/not right \(or the account is locked/)).toBeInTheDocument()
    expect(screen.getByLabelText('Password')).toHaveValue('')
    expect(screen.getByLabelText('One-time code')).toHaveValue('')
  })

  it('explains why a passkey cannot be used on a bare address and offers codes', () => {
    render(<RemoteSignIn options={{ ...options, passkey: false, passkey_problem: 'Passkeys need the panel to be opened by a name, not an address like 192.168.1.5.' }} onSignedIn={() => undefined} />)
    expect(screen.getByLabelText('Password')).toBeInTheDocument()
  })

  it('signs in with a passkey: asks for a challenge, signs it in the browser, sends it back', async () => {
    const user = userEvent.setup()
    const { get } = withPasskeys()
    const raw = new Uint8Array([1, 2, 3]).buffer
    get.mockResolvedValue({ id: 'abc', rawId: raw, type: 'public-key', getClientExtensionResults: () => ({}), response: { authenticatorData: raw, clientDataJSON: raw, signature: raw, userHandle: null } })
    fetchMock
      .mockResolvedValueOnce(jsonResponse(200, { step: 'challenge', handle: 'h1', options: { publicKey: { challenge: 'AQID', rp: { id: 'box.lan' }, allowCredentials: [{ id: 'AQID', type: 'public-key' }] } } }))
      .mockResolvedValueOnce(jsonResponse(200, { step: 'signed_in', identity: { name: 'sam', provider: 'passkey', role: 'admin' }, secret: 'k', notice: null }))
    const done = vi.fn()
    render(<RemoteSignIn options={options} onSignedIn={done} />)
    await user.type(screen.getByLabelText('Your name'), 'sam')
    await user.click(screen.getByRole('button', { name: 'Sign in with a passkey' }))
    await waitFor(() => expect(done).toHaveBeenCalled())
    expect(body(0)).toEqual({ step: 'passkey_begin', user: 'sam' })
    // The browser was handed ArrayBuffers, not strings.
    const pk = get.mock.calls[0][0].publicKey
    expect(pk.challenge).toBeInstanceOf(Uint8Array)
    expect(Array.from(pk.challenge)).toEqual([1, 2, 3])
    expect(pk.allowCredentials[0].id).toBeInstanceOf(Uint8Array)
    // The answer goes back as base64url text.
    expect(body(1)).toMatchObject({ step: 'passkey_finish', handle: 'h1', credential: { id: 'abc', rawId: 'AQID', response: { signature: 'AQID', userHandle: null } } })
  })

  it('says what the browser said when the passkey prompt is cancelled', async () => {
    const user = userEvent.setup()
    const { get } = withPasskeys()
    get.mockRejectedValue(new DOMException('x', 'NotAllowedError'))
    fetchMock.mockResolvedValueOnce(jsonResponse(200, { step: 'challenge', handle: 'h1', options: { publicKey: { challenge: 'AQID' } } }))
    render(<RemoteSignIn options={options} onSignedIn={() => undefined} />)
    await user.type(screen.getByLabelText('Your name'), 'sam')
    await user.click(screen.getByRole('button', { name: 'Sign in with a passkey' }))
    expect(await screen.findByText(/closed, timed out or refused/)).toBeInTheDocument()
  })
})

describe('the enrolment page', () => {
  it('sets up one-time codes, shows the recovery codes once and signs in', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (_u: string, init: RequestInit) => {
      const b = JSON.parse(String(init.body))
      switch (b.step) {
        case 'check':
          return jsonResponse(200, { step: 'who', user: 'amy', role: 'operator', passkey: false, passkey_problem: 'Passkeys need the panel to be opened by a name.' })
        case 'code_begin':
          return jsonResponse(200, { step: 'code_setup', secret: 'JBSWY3DPEHPK3PXP', otpauth: 'otpauth://totp/x', qr_svg: '<svg xmlns="http://www.w3.org/2000/svg"/>' })
        default:
          return jsonResponse(200, { step: 'done', identity: { name: 'amy', provider: 'code', role: 'operator' }, secret: 's', recovery_codes: ['aaaaa-bbbbb', 'ccccc-ddddd'] })
      }
    })
    const done = vi.fn()
    render(<Enrol link="L1" onDone={done} />)
    expect(await screen.findByText('Welcome, amy')).toBeInTheDocument()
    expect(screen.getByText(/Passkeys need the panel to be opened by a name/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Set up a passkey' })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Set up one-time codes' }))
    expect(await screen.findByAltText('QR code for your authenticator app')).toBeInTheDocument()
    await user.type(screen.getByLabelText('Choose a password'), 'plum otter lantern quietly')
    await user.type(screen.getByLabelText('Enter the 6-digit code the app shows'), '123456')
    await user.click(screen.getByRole('button', { name: 'Finish set-up' }))
    expect(await screen.findByText('aaaaa-bbbbb')).toBeInTheDocument()
    expect(done).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: 'I have saved them, carry on' }))
    expect(done).toHaveBeenCalled()
  })

  it('says so when the link was already used', async () => {
    fetchMock.mockResolvedValue(jsonResponse(401, { error: { code: 'unauthorized', message: 'This link has expired or was already used. Ask the person who runs Kora for a new one.' } }))
    render(<Enrol link="gone" onDone={() => undefined} />)
    expect(await screen.findByText(/expired or was already used/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Set up/ })).not.toBeInTheDocument()
  })
})

describe('the fresh check', () => {
  it('asks for a code, then repeats the request that needed it, once', async () => {
    const user = userEvent.setup()
    let deletes = 0
    fetchMock.mockImplementation(async (url: string) => {
      if (url.endsWith('/auth/reauth')) return jsonResponse(200, { step: 'fresh', valid_for: 600 })
      deletes++
      return deletes === 1 ? jsonResponse(403, { error: { code: 'remote.reauth_required', message: 'Confirm it is you first.' } }) : jsonResponse(200, { job: 'j1' })
    })
    const qc = new QueryClient()
    render(<QueryClientProvider client={qc}><ReauthProvider><p>app</p></ReauthProvider></QueryClientProvider>)
    const call = api.servers.delete('prod', { confirm_name: 'prod', keep_files: false, keep_db: false })
    expect(await screen.findByText('Confirm it is you')).toBeInTheDocument()
    await user.type(screen.getByLabelText('Password'), 'plum otter lantern quietly')
    await user.type(screen.getByLabelText('One-time code'), '123456')
    await user.click(screen.getByRole('button', { name: 'Confirm' }))
    await expect(call).resolves.toMatchObject({ job: 'j1' })
    expect(deletes).toBe(2)
    await waitFor(() => expect(screen.queryByText('Confirm it is you')).not.toBeInTheDocument())
  })

  it('gives up when the person closes the dialog (the original error is shown, no retry)', async () => {
    const user = userEvent.setup()
    fetchMock.mockResolvedValue(jsonResponse(403, { error: { code: 'remote.reauth_required', message: 'Confirm it is you first.' } }))
    const qc = new QueryClient()
    render(<QueryClientProvider client={qc}><ReauthProvider><p>app</p></ReauthProvider></QueryClientProvider>)
    const outcome = api.servers.unregister('prod').then(() => null, (e: unknown) => e)
    await screen.findByText('Confirm it is you')
    await user.keyboard('{Escape}')
    expect(await outcome).toBeInstanceOf(ApiError)
    expect(fetchMock).toHaveBeenCalledTimes(1)
  })

  it('does not loop: a second refusal after a good check is shown', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (url: string) => (url.endsWith('/auth/reauth') ? jsonResponse(200, { step: 'fresh', valid_for: 600 }) : jsonResponse(403, { error: { code: 'remote.reauth_required', message: 'again' } })))
    const qc = new QueryClient()
    render(<QueryClientProvider client={qc}><ReauthProvider><p>app</p></ReauthProvider></QueryClientProvider>)
    const outcome = api.servers.unregister('prod').then(() => null, (e: unknown) => e)
    await screen.findByText('Confirm it is you')
    await user.type(screen.getByLabelText('Password'), 'x')
    await user.type(screen.getByLabelText('One-time code'), '1')
    await user.click(screen.getByRole('button', { name: 'Confirm' }))
    expect(await outcome).toMatchObject({ code: 'remote.reauth_required' })
    expect(fetchMock.mock.calls.filter(([u]) => !String(u).endsWith('/auth/reauth')).length).toBe(2)
  })
})

describe('your sign-in card', () => {
  const account = (over: object) => ({
    user: 'sam', provider: 'token', role: 'admin', passkeys: [], code: false, recovery_left: 0, legacy_token: true, strong: false, passkey_problem: null, upgrade_suggested: true,
    devices: [{ handle: 'd1', device: 'Chrome on Linux', address: '10.0.0.2', signed_in: 1, last_seen: 2, current: true }], ...over,
  })
  const renderCard = () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    return render(<QueryClientProvider client={qc}><AccountCard /></QueryClientProvider>)
  }

  it('tells a token user to replace the link, and will not end it before something stronger exists', async () => {
    fetchMock.mockResolvedValue(jsonResponse(200, account({})))
    renderCard()
    expect(await screen.findByText('Replace your token link')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'End my token link' })).toBeDisabled()
    expect(screen.getByText('this device')).toBeInTheDocument()
  })

  it('lists passkeys and refuses nothing silently: the panel\'s reason is shown', async () => {
    const user = userEvent.setup()
    fetchMock.mockImplementation(async (_u: string, init?: RequestInit) => {
      if (init?.method === 'POST') return jsonResponse(409, { error: { code: 'conflict', message: 'This is your only way to sign in. Add another passkey or set up one-time codes first.' } })
      return jsonResponse(200, account({ provider: 'passkey', upgrade_suggested: false, strong: true, legacy_token: false, passkeys: [{ id: 7, label: 'Laptop', rp_id: 'box.lan', created: 1, last_used: 0 }] }))
    })
    renderCard()
    await user.click(await screen.findByRole('button', { name: 'Remove passkey Laptop' }))
    await user.click(screen.getByRole('button', { name: 'Remove' }))
    expect(await screen.findByText(/only way to sign in/)).toBeInTheDocument()
    expect(screen.queryByText('Replace your token link')).not.toBeInTheDocument()
  })
})
