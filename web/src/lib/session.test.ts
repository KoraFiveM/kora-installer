import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { bootstrapSession, tokenFromHash } from './session'
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

describe('bootstrapSession', () => {
  it('does nothing without a token', async () => {
    expect(await bootstrapSession()).toEqual({ status: 'none' })
    expect(fetchMock).not.toHaveBeenCalled()
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
