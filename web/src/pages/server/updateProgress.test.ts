import { describe, expect, it } from 'vitest'
import { applyProgress, checkProgress, rowCheckProgress } from './updateProgress'
import { percentOf } from '@/components/JobProgress'

const checked = (done: number, total: number, id: string) => ({ type: 'checked', done, total, id })
const planned = (...ids: string[]) => ({ type: 'planned', ids })
const res = (id: string, state: string) => ({ type: 'resource', id, state })

describe('checkProgress', () => {
  it('is undefined before the first checked event, then the latest one', () => {
    expect(checkProgress([])).toBeUndefined()
    expect(checkProgress([{ type: 'progress', message: 'x' }])).toBeUndefined()
    expect(checkProgress([checked(1, 3, 'kora_core'), checked(2, 3, 'kora_hud')])).toEqual({ done: 2, total: 3, id: 'kora_hud' })
  })

  it('reaches the total in order (100%)', () => {
    const events = [checked(1, 3, 'a'), checked(2, 3, 'b'), checked(3, 3, 'c')]
    const p = checkProgress(events)!
    expect(p).toEqual({ done: 3, total: 3, id: 'c' })
    expect(percentOf(p.done, p.total)).toBe(100)
  })

  it('a row check shows only on the row being checked', () => {
    const events = [checked(1, 1, 'kora_hud')]
    expect(rowCheckProgress('kora_hud', 'kora_hud', events)).toEqual({ done: 1, total: 1, id: 'kora_hud' })
    expect(rowCheckProgress('kora_core', 'kora_hud', events)).toBeUndefined()
    expect(rowCheckProgress('kora_core', undefined, events)).toBeUndefined()
    // Waiting for the answer: the row shows 0 of 1 rather than nothing.
    expect(rowCheckProgress('kora_hud', 'kora_hud', [])).toEqual({ done: 0, total: 1, id: '' })
  })
})

describe('applyProgress', () => {
  it('is undefined without a plan', () => {
    expect(applyProgress([])).toBeUndefined()
    expect(applyProgress([res('a', 'updated')])).toBeUndefined()
  })

  it('moves in order to 100% as each resource finishes', () => {
    const ev = [planned('a', 'b', 'c'), res('a', 'updating'), res('a', 'updated'), res('b', 'updating'), res('b', 'updated'), res('c', 'updating'), res('c', 'updated')]
    const seen = ev.map((_, n) => applyProgress(ev.slice(0, n + 1))!.done)
    expect(seen).toEqual([0, 0, 1, 1, 2, 2, 3])
    const last = applyProgress(ev)!
    expect(last).toEqual({ total: 3, done: 3, failed: 0, current: undefined })
    expect(percentOf(last.done, last.total)).toBe(100)
  })

  it('a failure counts as done and does not stall the bar', () => {
    const ev = [planned('a', 'b', 'c'), res('a', 'updated'), res('b', 'updating'), res('b', 'failed: fast-forward failed'), res('c', 'skipped: has changes of its own')]
    const p = applyProgress(ev)!
    expect(p).toEqual({ total: 3, done: 3, failed: 1, current: undefined })
    expect(percentOf(p.done, p.total)).toBe(100)
  })

  it('a resource that is still moving is not done and is what is happening now', () => {
    const p = applyProgress([planned('a', 'b'), res('a', 'updated'), res('b', 'updating')])!
    expect(p).toEqual({ total: 2, done: 1, failed: 0, current: 'Updating b' })
  })

  it('a rollback does not take a finished resource back out of the count', () => {
    const ev = [planned('a'), res('a', 'updating'), res('a', 'updated'), res('a', 'rolling back'), res('a', 'rolled back: health check failed')]
    expect(applyProgress(ev)!.done).toBe(1)
    expect(applyProgress(ev.slice(0, 4))!.current).toBe('Rolling back a')
    expect(applyProgress(ev.slice(0, 4))!.done).toBe(1)
  })

  it('ignores events for resources that were not planned', () => {
    expect(applyProgress([planned('a'), res('zzz', 'updated')])!.done).toBe(0)
  })
})
