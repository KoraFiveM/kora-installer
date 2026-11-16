import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import type { ReactNode } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { useServerEvents } from './useServerEvents'
import type { ServerSummary } from '@/api/types'
import { FakeEventSource } from '@/test/utils'

beforeEach(() => {
  FakeEventSource.instances = []
  vi.stubGlobal('EventSource', FakeEventSource)
})
afterEach(() => vi.unstubAllGlobals())

const srv = (name: string, status: string) => ({ name, status, badge: null }) as unknown as ServerSummary

describe('useServerEvents', () => {
  it('takes the whole list the panel sends and refreshes open server pages when a status changes', () => {
    const qc = new QueryClient()
    qc.setQueryData(['servers'], [srv('a', 'running'), srv('b', 'stopped')])
    const invalidate = vi.spyOn(qc, 'invalidateQueries')
    renderHook(() => useServerEvents(), { wrapper: ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider> })
    const es = FakeEventSource.instances[0]
    // The first message repeats what is known: nothing to refresh.
    act(() => es.emit('servers', { servers: [srv('a', 'running'), srv('b', 'stopped')] }))
    expect(invalidate).not.toHaveBeenCalled()
    act(() => es.emit('servers', { servers: [srv('a', 'stopped'), srv('b', 'stopped')] }))
    expect(qc.getQueryData<ServerSummary[]>(['servers'])?.[0].status).toBe('stopped')
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['server'] })
  })
})
