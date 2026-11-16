import { describe, expect, it } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import { clearKept, useKept } from './kept'

describe('useKept', () => {
  it('keeps a value after the component has gone, per key', () => {
    const a = renderHook(() => useKept('a', 0))
    act(() => a.result.current[1]((n) => n + 2))
    expect(a.result.current[0]).toBe(2)
    a.unmount()
    expect(renderHook(() => useKept('a', 0)).result.current[0]).toBe(2)
    expect(renderHook(() => useKept('b', 0)).result.current[0]).toBe(0)
  })

  it('updates every component showing the same key', () => {
    const one = renderHook(() => useKept<string | null>('k', null))
    const two = renderHook(() => useKept<string | null>('k', null))
    act(() => one.result.current[1]('job'))
    expect(two.result.current[0]).toBe('job')
  })

  it('starts fresh after clearKept, without looping on a new initial object each render', () => {
    const h = renderHook(() => useKept('s', new Set<string>()))
    act(() => h.result.current[1](new Set(['x'])))
    act(() => clearKept())
    expect(h.result.current[0].size).toBe(0)
  })
})
