import { useMemo, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ResourceChange, RestartPolicy, UpdatePreview, UpdateReport } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useKept } from '@/lib/kept'

/** Put the freshly checked rows into the list; every other row stays exactly as it was. A row that is not in the list yet is added. */
export function mergeRows(view: UpdatePreview, fresh: ResourceChange[]): UpdatePreview {
  const byId = new Map(fresh.map((r) => [r.id, r]))
  const known = new Set(view.resources.map((r) => r.id))
  return { ...view, resources: [...view.resources.map((r) => byId.get(r.id) ?? r), ...fresh.filter((r) => !known.has(r.id))] }
}

const EMPTY: UpdatePreview = { resources: [], new_release: null }

/**
 * Everything the Resources tab knows about updates for one server: the running check, what it found,
 * what is ticked, the update and rollback in progress. It is kept for the server (not for the tab on
 * screen), so opening another tab and coming back shows the same thing, including a check still running.
 * The server runs one update job at a time, so `busy` covers them all.
 */
export function useUpdates(server: string) {
  const qc = useQueryClient()
  const k = (part: string) => `updates:${server}:${part}`
  const [checkJob, setCheckJob] = useKept<string | null>(k('check'), null)
  const [rowJob, setRowJob] = useKept<{ id: string; job: string } | null>(k('row'), null)
  const [applyJob, setApplyJob] = useKept<string | null>(k('apply'), null)
  const [rollJob, setRollJob] = useKept<string | null>(k('roll'), null)
  const [view, setView] = useKept<UpdatePreview | null>(k('view'), null)
  // True once every resource has been checked together (only then can "everything is up to date" be said).
  const [full, setFull] = useKept(k('full'), false)
  const [selected, setSelected] = useKept<Set<string>>(k('selected'), new Set())
  const [restart, setRestart] = useKept<RestartPolicy>(k('restart'), 'when_quiet')
  const [checkedAt, setCheckedAt] = useKept<Record<string, number>>(k('checkedAt'), {})
  const [lastRollback, setLastRollback] = useKept<string | null>(k('lastRollback'), null)
  // A request that was refused before any job started. Not kept: it describes the click just made.
  const [startError, setStartError] = useState<unknown>(null)

  const stamp = (rows: ResourceChange[]) => Object.fromEntries(rows.map((r) => [r.id, Date.now()]))

  const check = useJob<UpdatePreview>(checkJob, (j) => {
    if (j.state === 'done' && j.result) {
      const res = j.result
      setView(res)
      setFull(true)
      setSelected(new Set(res.resources.filter((r) => r.status === 'clean').map((r) => r.id)))
      setCheckedAt(stamp(res.resources))
    }
    void qc.invalidateQueries({ queryKey: ['servers'] })
    void qc.invalidateQueries({ queryKey: ['server', server] })
  })
  // One resource: only that row changes. A row that has just become ready to update is ticked; what
  // the person unticked stays unticked.
  const rowCheck = useJob<UpdatePreview>(rowJob?.job ?? null, (j) => {
    if (j.state !== 'done' || !j.result) return
    const fresh = j.result.resources
    setView((v) => {
      const before = new Map((v?.resources ?? []).map((r) => [r.id, r.status]))
      setSelected((x) => {
        const n = new Set(x)
        for (const r of fresh) {
          if (r.status !== 'clean') n.delete(r.id)
          else if (before.get(r.id) !== 'clean') n.add(r.id)
        }
        return n
      })
      return mergeRows(v ?? EMPTY, fresh)
    })
    setCheckedAt((c) => ({ ...c, ...stamp(fresh) }))
    void qc.invalidateQueries({ queryKey: ['server', server] })
  })
  const apply = useJob<UpdateReport>(applyJob, (j) => {
    // What was updated is now up to date: say so in the list instead of offering it again.
    if (j.state === 'done' && j.result) {
      const done = new Map(j.result.updated.map((u) => [u.id, u.to]))
      setView((v) => v && { ...v, resources: v.resources.map((r) => (done.has(r.id) ? { ...r, status: 'up_to_date', from: done.get(r.id)!, behind: 0, changelog: [] } : r)) })
      setSelected((x) => new Set([...x].filter((id) => !done.has(id))))
    }
    void qc.invalidateQueries({ queryKey: ['snapshots', server] })
    void qc.invalidateQueries({ queryKey: ['servers'] })
    void qc.invalidateQueries({ queryKey: ['server', server] })
  })
  const roll = useJob(rollJob, (j) => {
    // The resources are back at earlier versions, so what the last check found no longer holds.
    if (j.state === 'done') {
      setView(null)
      setFull(false)
      setSelected(new Set())
      setCheckedAt({})
    }
    void qc.invalidateQueries({ queryKey: ['snapshots', server] })
    void qc.invalidateQueries({ queryKey: ['servers'] })
    void qc.invalidateQueries({ queryKey: ['server', server] })
  })

  const rows = useMemo(() => view?.resources ?? [], [view])
  const clean = useMemo(() => rows.filter((r) => r.status === 'clean'), [rows])
  // What the Update button will update: ticked, and still ready.
  const chosen = useMemo(() => clean.filter((r) => selected.has(r.id)), [clean, selected])
  const busy = check.phase === 'running' || rowCheck.phase === 'running' || apply.phase === 'running' || roll.phase === 'running'

  // Between the click and the panel's answer nothing is "running" yet: a second click must not start a second job.
  const starting = useRef(false)
  const start = async (run: () => Promise<void>) => {
    if (starting.current || busy) return
    starting.current = true
    setStartError(null)
    try {
      await run()
    } catch (e) {
      setStartError(e)
    } finally {
      starting.current = false
    }
  }

  return {
    view,
    full,
    rows,
    clean,
    chosen,
    byId: useMemo(() => new Map(rows.map((r) => [r.id, r])), [rows]),
    selected,
    restart,
    setRestart,
    checkedAt,
    busy,
    check,
    rowCheck,
    apply,
    roll,
    lastRollback,
    /** The resource being checked on its own right now. */
    checkingRow: rowCheck.phase === 'running' ? rowJob?.id : undefined,
    /** Why the last click did nothing, or why the last check failed. */
    error: startError ?? (check.phase === 'failed' ? check.error : rowCheck.phase === 'failed' ? rowCheck.error : null),
    toggle: (id: string) => setSelected((x) => { const n = new Set(x); if (n.has(id)) n.delete(id); else n.add(id); return n }),
    selectAll: () => setSelected(new Set(clean.map((r) => r.id))),
    selectNone: () => setSelected(new Set()),
    checkAll: () => start(async () => {
      setApplyJob(null)
      setCheckJob((await api.update.preview(server)).job)
    }),
    checkOne: (id: string) => start(async () => setRowJob({ id, job: (await api.update.preview(server, { resources: [id] })).job })),
    applySelected: () => start(async () => {
      const skip = rows.filter((r) => !selected.has(r.id)).map((r) => r.id)
      // Exactly the versions shown, even if something newer was published since the check.
      const targets = Object.fromEntries(chosen.map((r) => [r.id, r.to]))
      setApplyJob((await api.update.apply(server, { scope: { resources: Object.keys(targets) }, restart, skip, targets })).job)
    }),
    /** Update this one resource to the version the last check showed (no restart: the resource page offers it). */
    applyOne: (id: string) => start(async () => {
      const row = rows.find((r) => r.id === id)
      if (!row || row.status !== 'clean') return
      setApplyJob((await api.update.apply(server, { scope: { resources: [id] }, restart: 'none', skip: [], targets: { [id]: row.to } })).job)
    }),
    rollback: (snapshot: string, force: boolean) => start(async () => {
      setLastRollback(snapshot)
      setRollJob((await api.snapshots.rollback(server, snapshot, force)).job)
    }),
  }
}

export type Updates = ReturnType<typeof useUpdates>
