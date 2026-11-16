import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { restartPanel } from './restart'
import { jsonResponse } from '@/test/utils'

const fetchMock = vi.fn()
beforeEach(() => { vi.useFakeTimers(); fetchMock.mockReset(); vi.stubGlobal('fetch', fetchMock) })
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals() })

const meta = (version: string) => jsonResponse(200, { version, api: 1 })

describe('restartPanel', () => {
  it('posts the restart and resolves once the panel answers with a new version', async () => {
    fetchMock
      .mockResolvedValueOnce(meta('0.4.1'))
      .mockResolvedValueOnce(jsonResponse(202, { restarting: true }))
      .mockResolvedValueOnce(meta('0.4.1'))
      .mockResolvedValueOnce(meta('0.4.2'))
    const p = restartPanel()
    await vi.advanceTimersByTimeAsync(2_000)
    await expect(p).resolves.toBe(true)
    const [url, init] = fetchMock.mock.calls[1]
    expect(url).toBe('/api/v1/panel/restart')
    expect(init.method).toBe('POST')
  })

  it('resolves when the panel answers again after being down', async () => {
    fetchMock
      .mockResolvedValueOnce(meta('0.4.1'))
      .mockResolvedValueOnce(jsonResponse(202, { restarting: true }))
      .mockRejectedValueOnce(new TypeError('Failed to fetch'))
      .mockResolvedValueOnce(meta('0.4.1'))
    const p = restartPanel()
    await vi.advanceTimersByTimeAsync(2_000)
    await expect(p).resolves.toBe(true)
  })

  it('gives up after the timeout', async () => {
    fetchMock
      .mockResolvedValueOnce(meta('0.4.1'))
      .mockResolvedValueOnce(jsonResponse(202, { restarting: true }))
      .mockImplementation(async () => meta('0.4.1'))
    const p = restartPanel()
    await vi.advanceTimersByTimeAsync(61_000)
    await expect(p).resolves.toBe(false)
  })

  it('rejects when the server refuses', async () => {
    fetchMock
      .mockResolvedValueOnce(meta('0.4.1'))
      .mockResolvedValueOnce(jsonResponse(409, { error: { message: 'no' } }))
    await expect(restartPanel()).rejects.toThrow()
  })
})
