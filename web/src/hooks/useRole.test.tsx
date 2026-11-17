import { describe, expect, it } from 'vitest'
import { renderHook } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import type { Meta } from '@/api/types'
import { useRole, whyNot } from './useRole'

function withMeta(meta: Partial<Meta> | undefined) {
  const qc = new QueryClient()
  if (meta) qc.setQueryData(['meta'], { version: '0', api: 1, platform: 'linux', ops: [], node: 'n', ...meta })
  return ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>
}

describe('useRole', () => {
  it('treats a missing identity as a local admin', () => {
    const { result } = renderHook(() => useRole(), { wrapper: withMeta(undefined) })
    expect(result.current.role).toBe('admin')
    expect(result.current.canDestroy).toBe(true)
  })

  it('a remote viewer can only look', () => {
    const { result } = renderHook(() => useRole(), { wrapper: withMeta({ remote: true, identity: { name: 'vera', provider: 'token', role: 'viewer' } }) })
    expect(result.current.can('viewer')).toBe(true)
    expect(result.current.can('operator')).toBe(false)
    expect(whyNot(result.current, 'operator')).toMatch(/viewer/)
  })

  it('a remote admin needs the destructive switch', () => {
    const id = { name: 'adam', provider: 'tailscale', role: 'admin' } as const
    const off = renderHook(() => useRole(), { wrapper: withMeta({ remote: true, identity: id }) }).result.current
    expect(off.can('admin')).toBe(true)
    expect(off.canDestroy).toBe(false)
    expect(whyNot(off, 'admin', true)).toMatch(/turned off for sign-ins made with a token.*kora panel remote-actions on/)
    const on = renderHook(() => useRole(), { wrapper: withMeta({ remote: true, identity: id, allow_remote_destructive: true }) }).result.current
    expect(on.canDestroy).toBe(true)
    expect(whyNot(on, 'admin', true)).toBeNull()
  })
})
