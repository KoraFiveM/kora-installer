import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, renderHook, waitFor } from '@testing-library/react'
import { JOB_POLL_MS, useJob } from './useJob'
import { FakeEventSource, jsonResponse } from '@/test/utils'

const fetchMock = vi.fn()
beforeEach(() => {
  FakeEventSource.instances = []
  fetchMock.mockReset()
  vi.stubGlobal('fetch', fetchMock)
  vi.stubGlobal('EventSource', FakeEventSource)
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

const job = (state: string, extra: object = {}) => ({ id: 'j1', op: 'x', server: null, state, events: [], result: null, error: null, ...extra })

describe('useJob', () => {
  it('stays idle without a job id', () => {
    const { result } = renderHook(() => useJob(null))
    expect(result.current.phase).toBe('idle')
    expect(FakeEventSource.instances).toHaveLength(0)
  })

  it('collects progress events and takes the result from the job record when it is done', async () => {
    fetchMock.mockResolvedValue(jsonResponse(200, job('done', { result: { dir: '/x' } })))
    const onFinish = vi.fn()
    const { result } = renderHook(() => useJob<{ dir: string }, { n: number }>('j1', onFinish))
    const es = FakeEventSource.instances[0]
    expect(es.url).toBe('/api/v1/jobs/j1/events')
    act(() => es.emit('progress', { n: 1 }))
    act(() => es.emit('progress', { n: 2 }))
    expect(result.current.phase).toBe('running')
    expect(result.current.events).toEqual([{ n: 1 }, { n: 2 }])
    act(() => es.emit('done', {}))
    await waitFor(() => expect(result.current.phase).toBe('done'))
    expect(result.current.result).toEqual({ dir: '/x' })
    expect(es.closed).toBe(true)
    expect(onFinish).toHaveBeenCalledTimes(1)
  })

  it('reports a failed job with its error', async () => {
    fetchMock.mockResolvedValue(jsonResponse(200, job('failed', { error: { code: 'internal', message: 'The database said no.' } })))
    const { result } = renderHook(() => useJob('j1'))
    act(() => FakeEventSource.instances[0].emit('failed', {}))
    await waitFor(() => expect(result.current.phase).toBe('failed'))
    expect(result.current.error?.message).toBe('The database said no.')
  })

  it('falls back to polling when the event stream breaks', async () => {
    vi.useFakeTimers()
    fetchMock
      .mockResolvedValueOnce(jsonResponse(200, job('running', { events: [{ n: 1 }] })))
      .mockResolvedValueOnce(jsonResponse(200, job('done', { events: [{ n: 1 }, { n: 2 }], result: { ok: true } })))
    const { result } = renderHook(() => useJob('j1'))
    await act(async () => { FakeEventSource.instances[0].fail() })
    expect(result.current.phase).toBe('running')
    expect(result.current.events).toEqual([{ n: 1 }])
    await act(async () => { await vi.advanceTimersByTimeAsync(JOB_POLL_MS + 10) })
    expect(result.current.phase).toBe('done')
    expect(result.current.events).toHaveLength(2)
    expect(fetchMock).toHaveBeenCalledTimes(2)
  })

  it('closes the stream when unmounted', () => {
    const { unmount } = renderHook(() => useJob('j1'))
    const es = FakeEventSource.instances[0]
    unmount()
    expect(es.closed).toBe(true)
  })
})
