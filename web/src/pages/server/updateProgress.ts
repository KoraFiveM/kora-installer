import { normaliseUpdateEvent } from '@/components/JobProgress'
export { percentOf } from '@/components/JobProgress'

/** The latest "checked" event of a check job: `done` of `total` resources looked at; `id` is the one just finished. */
export interface CheckProgress {
  done: number
  total: number
  id: string
}

/** Where an update is: how many of the planned resources have finished, how many failed, and what is moving now. */
export interface ApplyProgress {
  total: number
  done: number
  failed: number
  /** What is happening right now, e.g. "Updating kora_core". */
  current?: string
}

/** Resource states that mean "not finished yet". Everything else (updated, failed, skipped, waiting, rolled back...) is final. */
const MOVING = new Set(['updating', 'rolling back'])

/** The latest checked event, or undefined before the first one arrives. */
export function checkProgress(events: unknown[]): CheckProgress | undefined {
  let last: CheckProgress | undefined
  for (const raw of events) {
    const e = normaliseUpdateEvent(raw) as { type?: string; done?: number; total?: number; id?: string }
    if (e.type === 'checked' && typeof e.done === 'number' && typeof e.total === 'number') {
      last = { done: e.done, total: e.total, id: e.id ?? '' }
    }
  }
  return last
}

/**
 * The progress of one resource's check. Only the row being checked gets one: every other row gets none.
 */
export function rowCheckProgress(rowId: string, checkingRow: string | undefined, events: unknown[]): CheckProgress | undefined {
  return checkingRow === rowId ? checkProgress(events) ?? { done: 0, total: 1, id: '' } : undefined
}

/**
 * Progress of an update. The total is the planned list (the `planned` event, sent before anything moves). A resource
 * counts as done once it has any final state, so a failed one (or one that was never tried) does not hold the bar back.
 * Returns undefined when no plan has arrived (an older panel, or nothing to update).
 */
export function applyProgress(events: unknown[]): ApplyProgress | undefined {
  let planned: string[] | undefined
  for (const raw of events) {
    const e = raw as { type?: string; ids?: unknown }
    if (e && typeof e === 'object' && e.type === 'planned' && Array.isArray(e.ids)) planned = e.ids.filter((x): x is string => typeof x === 'string')
  }
  if (!planned || planned.length === 0) return undefined
  const want = new Set(planned)
  const latest = new Map<string, string>()
  const finished = new Set<string>()
  // The last step that started and has not finished yet is what is happening now.
  let moving: { id: string; state: string } | undefined
  for (const raw of events) {
    const e = normaliseUpdateEvent(raw)
    if (e.type !== 'resource' || !e.id || !want.has(e.id)) continue
    const state = (e.state ?? '').toLowerCase()
    latest.set(e.id, state)
    if (MOVING.has(state)) moving = { id: e.id, state }
    else finished.add(e.id)
  }
  const failed = [...latest.values()].filter((s) => s.startsWith('failed')).length
  const current = moving && latest.get(moving.id) === moving.state ? `${moving.state === 'updating' ? 'Updating' : 'Rolling back'} ${moving.id}` : undefined
  return { total: planned.length, done: finished.size, failed, current }
}
