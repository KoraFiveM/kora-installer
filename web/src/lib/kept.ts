import { useCallback, useRef, useSyncExternalStore } from 'react'

// Page state that outlives the component showing it. A server page only mounts the open tab, so
// plain `useState` forgets a running check or a finished result the moment another tab is opened.
// Values live for as long as the page is loaded (a reload starts fresh).
const values = new Map<string, unknown>()
const listeners = new Map<string, Set<() => void>>()

/** `useState` whose value is kept under `key` while the page is loaded, whatever is on screen. */
export function useKept<T>(key: string, initial: T): [T, (next: T | ((prev: T) => T)) => void] {
  // One stable fallback per hook, so an `initial` built during render (`new Set()`) does not re-render forever.
  const fallback = useRef(initial)
  const subscribe = useCallback((fn: () => void) => {
    const set = listeners.get(key) ?? new Set()
    listeners.set(key, set)
    set.add(fn)
    return () => {
      set.delete(fn)
      if (set.size === 0) listeners.delete(key)
    }
  }, [key])
  const read = useCallback(() => (values.has(key) ? (values.get(key) as T) : fallback.current), [key])
  const value = useSyncExternalStore(subscribe, read, read)
  const set = useCallback((next: T | ((prev: T) => T)) => {
    const prev = values.has(key) ? (values.get(key) as T) : fallback.current
    const v = typeof next === 'function' ? (next as (p: T) => T)(prev) : next
    if (Object.is(v, prev) && values.has(key)) return
    values.set(key, v)
    listeners.get(key)?.forEach((fn) => fn())
  }, [key])
  return [value, set]
}

/** Forget everything (signing out, and between tests). */
export function clearKept(): void {
  values.clear()
  listeners.forEach((set) => set.forEach((fn) => fn()))
}
