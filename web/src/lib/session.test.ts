import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { bootstrapSession, codeFromHash, pastedSecret, signIn, tokenFromHash } from './session'
import { sessionSecret } from '@/api/client'
import { jsonResponse } from '@/test/utils'

const fetchMock = vi.fn()
beforeEach(() => {
  fetchMock.mockReset()
  vi.stubGlobal('fetch', fetchMock)
  history.replaceState(null, '', '/')
})
afterEach(() => vi.unstubAllGlobals())

describe('tokenFromHash', () => {
  it('reads the token', () => expect(tokenFromHash('#token=abc123')).toBe('abc123'))
  it('ignores other hashes', () => {
    expect(tokenFromHash('')).toBeNull()
    expect(tokenFromHash('#section')).toBeNull()
    expect(tokenFromHash('#token=')).toBeNull()
  })
})

describe('codeFromHash', () => {
  it('reads the code', () => expect(codeFromHash('#code=ab12')).toBe('ab12'))
  it('ignores other hashes', () => {
    expect(codeFromHash('#token=abc')).toBeNull()
    expect(codeFromHash('#code=')).toBeNull()
  })
})

const meta = (over: object) => jsonResponse(200, { version: '0.5.1', api: 1, platform: 'linux', ops: [], node: 'n', identity: null, remote: false, ...over })
const signedIn = { ok: true, identity: { name: 'me@example.com', provider: 'tailscale', role: 'viewer' }, secret: 'sec' }

describe('bootstrapSession', () => {
  it('on this computer, without a link, only asks /meta', async () => {
    fetchMock.mockResolvedValue(meta({}))
    expect(await bootstrapSession()).toEqual({ status: 'none' })
    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/meta')
  })

  it('exchanges a one-time code from the link, keeps the secret and clears the address bar', async () => {
    history.replaceState(null, '', '/settings#code=0123abcd')
    fetchMock.mockResolvedValue(jsonResponse(200, { ...signedIn, secret: 'from-code' }))
    const r = await bootstrapSession()
    expect(r.status).toBe('signed_in')
    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('/api/v1/session')
    expect(JSON.parse(init.body)).toEqual({ code: '0123abcd' })
    expect(window.location.hash).toBe('')
    expect(window.location.pathname).toBe('/settings')
    expect(sessionSecret()).toBe('from-code')
  })

  it('clears a used code from the address bar and says why it failed', async () => {
    history.replaceState(null, '', '/#code=used')
    fetchMock.mockResolvedValue(jsonResponse(401, { error: { code: 'unauthorized', message: 'That sign-in link has expired or was already used.' } }))
    const r = await bootstrapSession()
    expect(window.location.hash).toBe('')
    expect(r.status).toBe('failed')
  })

  it('on another computer, tries the Tailscale sign-in once by itself', async () => {
    fetchMock.mockResolvedValueOnce(meta({ remote: true })).mockResolvedValueOnce(jsonResponse(200, signedIn))
    expect(await bootstrapSession()).toEqual({ status: 'signed_in' })
    expect(fetchMock).toHaveBeenCalledTimes(2)
    expect(JSON.parse(fetchMock.mock.calls[1][1].body)).toEqual({})
    expect(sessionSecret()).toBe('sec')
  })

  it('shows the sign-in screen, without an error, when Tailscale does not let them in', async () => {
    fetchMock.mockResolvedValueOnce(meta({ remote: true })).mockResolvedValueOnce(jsonResponse(401, { error: { code: 'unauthorized', message: 'Sign in with your user token.' } }))
    expect(await bootstrapSession()).toEqual({ status: 'signed_out' })
  })

  it('does not sign in again when already signed in from another computer', async () => {
    fetchMock.mockResolvedValue(meta({ remote: true, identity: signedIn.identity }))
    expect(await bootstrapSession()).toEqual({ status: 'none' })
    expect(fetchMock).toHaveBeenCalledTimes(1)
  })

  it('posts the token, then removes it from the address bar', async () => {
    history.replaceState(null, '', '/servers/a?tab=logs#token=s3cret')
    fetchMock.mockResolvedValue(new Response(null, { status: 204 }))
    const r = await bootstrapSession()
    expect(r.status).toBe('signed_in')
    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('/api/v1/session')
    expect(init.method).toBe('POST')
    expect(init.headers['X-Kora']).toBe('1')
    expect(JSON.parse(init.body)).toEqual({ token: 's3cret' })
    expect(window.location.hash).toBe('')
    expect(window.location.pathname + window.location.search).toBe('/servers/a?tab=logs')
  })

  it('removes the token even when the server refuses it, and reports why', async () => {
    history.replaceState(null, '', '/#token=old')
    fetchMock.mockResolvedValue(jsonResponse(401, { error: { code: 'unauthorized', message: 'That sign-in link has expired.' } }))
    const r = await bootstrapSession()
    expect(window.location.hash).toBe('')
    expect(r.status).toBe('failed')
    if (r.status === 'failed') expect(r.error.message).toBe('That sign-in link has expired.')
  })
})

describe('the sign-in box', () => {
  it('takes a code, a token or a whole sign-in link', () => {
    expect(pastedSecret(' ab12 ')).toEqual({ token: 'ab12' })
    expect(pastedSecret('http://127.0.0.1:47120/#code=ab12')).toEqual({ code: 'ab12' })
    expect(pastedSecret('  http://localhost:47120/settings#code=ab12\n')).toEqual({ code: 'ab12' })
    expect(pastedSecret('http://127.0.0.1:47120/#token=launch')).toEqual({ token: 'launch' })
    expect(pastedSecret('http://127.0.0.1:47120/#code=')).toEqual({ token: 'http://127.0.0.1:47120/#code=' })
  })

  it('sends a pasted link as its code', async () => {
    fetchMock.mockResolvedValue(jsonResponse(200, { ...signedIn, secret: 'pasted' }))
    expect(await signIn('http://127.0.0.1:47120/#code=0123abcd')).toEqual({ status: 'signed_in' })
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ code: '0123abcd' })
    expect(sessionSecret()).toBe('pasted')
  })

  it('sends anything else as typed', async () => {
    fetchMock.mockResolvedValue(jsonResponse(200, signedIn))
    await signIn(' c0de ')
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ token: 'c0de' })
  })
})
