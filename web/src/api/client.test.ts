import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { api, ApiError, sessionSecret, setUnauthorizedHandler } from './client'
import { commandLine, toNotice } from '@/components/ErrorNotice'
import { jsonResponse } from '@/test/utils'

const fetchMock = vi.fn()
beforeEach(() => {
  fetchMock.mockReset()
  vi.stubGlobal('fetch', fetchMock)
})
afterEach(() => {
  setUnauthorizedHandler(null)
  vi.unstubAllGlobals()
})

describe('session secret', () => {
  it('keeps the secret from sign-in and sends it with every request and stream', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse(200, { ok: true, identity: { name: 'local', provider: 'local', role: 'admin' }, secret: 's3cr3t' }))
    await api.session.createByCode('c0de')
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ code: 'c0de' })
    expect(sessionSecret()).toBe('s3cr3t')
    expect(window.localStorage.getItem('kora_session_secret')).toBe('s3cr3t')
    fetchMock.mockImplementation(async () => jsonResponse(200, { servers: [] }))
    await api.servers.list()
    expect(fetchMock.mock.calls[1][1].headers['X-Kora-Session']).toBe('s3cr3t')
    await api.servers.control('a', 'start')
    expect(fetchMock.mock.calls[2][1].headers['X-Kora-Session']).toBe('s3cr3t')
    // EventSource cannot send headers: the streams carry it in the query.
    expect(api.eventsUrl()).toBe('/api/v1/events?ks=s3cr3t')
    expect(api.jobs.eventsUrl('j 1')).toBe('/api/v1/jobs/j%201/events?ks=s3cr3t')
    expect(api.console.streamUrl('my server')).toBe('/api/v1/servers/my%20server/console/stream?ks=s3cr3t')
  })

  it('sends nothing extra before sign-in', async () => {
    fetchMock.mockResolvedValue(jsonResponse(200, { servers: [] }))
    await api.servers.list()
    expect(fetchMock.mock.calls[0][1].headers['X-Kora-Session']).toBeUndefined()
    expect(api.eventsUrl()).toBe('/api/v1/events')
  })

  it('forgets the secret when the session ends or the user signs out', async () => {
    window.localStorage.setItem('kora_session_secret', 'old')
    fetchMock.mockResolvedValueOnce(jsonResponse(401, { error: { code: 'unauthorized', message: 'missing or invalid session' } }))
    await api.servers.list().catch(() => undefined)
    expect(sessionSecret()).toBeNull()
    window.localStorage.setItem('kora_session_secret', 'old')
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }))
    await api.session.logout()
    expect(sessionSecret()).toBeNull()
  })
})

describe('api client', () => {
  it('returns parsed JSON and sends cookies but no X-Kora on GET', async () => {
    fetchMock.mockResolvedValue(jsonResponse(200, { servers: [{ name: 'a' }] }))
    const list = await api.servers.list()
    expect(list).toEqual([{ name: 'a' }])
    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('/api/v1/servers')
    expect(init.credentials).toBe('same-origin')
    expect(init.headers['X-Kora']).toBeUndefined()
  })

  it('sends X-Kora on every non-GET request and percent-encodes server names', async () => {
    fetchMock.mockResolvedValue(jsonResponse(202, { job: 'j1' }))
    await api.servers.control('my server', 'restart')
    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('/api/v1/servers/my%20server/control')
    expect(init.method).toBe('POST')
    expect(init.headers['X-Kora']).toBe('1')
    expect(JSON.parse(init.body)).toEqual({ action: 'restart' })
  })

  it('turns an API error body into an ApiError with the message unchanged', async () => {
    fetchMock.mockResolvedValue(jsonResponse(409, { error: { code: 'conflict', message: 'Port 30120 is already in use.' } }))
    const err = await api.ports.set('a', { game: 1, txadmin: 2 }).catch((e) => e)
    expect(err).toBeInstanceOf(ApiError)
    expect(err.message).toBe('Port 30120 is already in use.')
    expect(err.code).toBe('conflict')
    expect(err.status).toBe(409)
  })

  it('keeps the command of a needs_elevation error', async () => {
    fetchMock.mockResolvedValue(jsonResponse(403, { error: { code: 'needs_elevation', message: 'Needs admin.', command: ['sudo', 'kora', 'schedule', 'my server'] } }))
    const err = (await api.schedule.remove('a').catch((e) => e)) as ApiError
    expect(err.needsElevation).toBe(true)
    expect(err.command).toEqual(['sudo', 'kora', 'schedule', 'my server'])
    expect(toNotice(err)?.command).toBeDefined()
    expect(commandLine(err.command!)).toBe('sudo kora schedule "my server"')
  })

  it('gives a plain message when the error body is not JSON', async () => {
    fetchMock.mockResolvedValue(new Response('<html>oops</html>', { status: 502 }))
    const err = (await api.meta().catch((e) => e)) as ApiError
    expect(err.status).toBe(502)
    expect(err.message).toMatch(/unexpected error/)
  })

  it('reports a network failure with a readable message', async () => {
    fetchMock.mockRejectedValue(new TypeError('Failed to fetch'))
    const err = (await api.meta().catch((e) => e)) as ApiError
    expect(err.code).toBe('network')
    expect(err.message).toMatch(/Could not reach Kora/)
  })

  it('calls the unauthorized handler on 401, except for the sign-in call itself', async () => {
    const handler = vi.fn()
    setUnauthorizedHandler(handler)
    fetchMock.mockResolvedValue(jsonResponse(401, { error: { code: 'unauthorized', message: 'Not signed in.' } }))
    await api.servers.list().catch(() => {})
    expect(handler).toHaveBeenCalledTimes(1)
    await api.session.create('bad').catch(() => {})
    expect(handler).toHaveBeenCalledTimes(1)
  })

  it('handles 204 with no body', async () => {
    fetchMock.mockResolvedValue(new Response(null, { status: 204 }))
    await expect(api.webhook.remove('a')).resolves.toBeUndefined()
  })
})
