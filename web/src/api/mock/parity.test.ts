// The mock backend answers the same routes as the real panel. The list of real routes is GENERATED from the Rust
// operations table (`generated/routes.ts`), so this test replaces a hand-kept list: add an operation in Rust,
// regenerate, and the mock has to learn it (or be listed here with a reason).
import { describe, expect, it } from 'vitest'
import { ROUTES } from '../generated/routes'
import { mockRouteKeys } from './index'

const shape = (method: string, path: string) => `${method} ${path.replace(/\{\w+\}/g, '{}')}`

/** Real operations the mock does not answer, with the reason. Shrinks as the mock learns them. */
const NOT_MOCKED: Record<string, string> = {
  session_create: 'sign-in is answered by the mock fetch itself, before the route table',
  console_stream: 'streams are played by the mock EventSource, not by a route',
  job_events: 'streams are played by the mock EventSource, not by a route',
  events: 'streams are played by the mock EventSource, not by a route',
}

describe('mock backend routes', () => {
  const mocked = new Set(mockRouteKeys().map((k) => shape(k.split(' ')[0], k.slice(k.indexOf(' ') + 1))))
  const real = new Map(Object.entries(ROUTES).map(([op, r]) => [shape(r.method, r.path), op]))

  it('answers every operation of the real panel', () => {
    const missing = [...real].filter(([k, op]) => !mocked.has(k) && !(op in NOT_MOCKED)).map(([k, op]) => `${op} (${k})`)
    expect(missing).toEqual([])
  })

  it('answers nothing the real panel does not have', () => {
    const extra = [...mocked].filter((k) => !real.has(k))
    expect(extra).toEqual([])
  })

  it('has no stale entry in the not-mocked list', () => {
    const stale = Object.keys(NOT_MOCKED).filter((op) => !(op in ROUTES) || mocked.has(shape(ROUTES[op as keyof typeof ROUTES].method, ROUTES[op as keyof typeof ROUTES].path)))
    expect(stale).toEqual([])
  })
})
