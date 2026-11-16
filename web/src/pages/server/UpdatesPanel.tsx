import { useEffect, useMemo, useState } from 'react'
import { Loader2, RefreshCw } from 'lucide-react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ChangeStatus, RestartPolicy, ResourceChange, ServerDetail, UpdatePreview, UpdateReport } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Radio } from '@/components/ui/checkbox'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { ErrorNotice } from '@/components/ErrorNotice'
import { normaliseUpdateEvent, ResourceProgress } from '@/components/JobProgress'
import { formatDate } from '@/lib/utils'
import { timeAgo } from '@/lib/ago'

export const RESTART_OPTIONS: { value: RestartPolicy; label: string; hint: string }[] = [
  { value: 'none', label: 'Do not restart', hint: 'Update the files only. The changes load the next time the server restarts.' },
  { value: 'ensure', label: 'Update files, reload later', hint: 'Update the files without restarting. Resources reload when someone runs "ensure" in the txAdmin console, or at the next restart.' },
  { value: 'full_restart', label: 'Restart the whole server', hint: 'Always restart after updating. Players are disconnected for a minute or so.' },
  { value: 'when_quiet', label: 'Restart only when nobody is playing', hint: 'If players are online, the restart is left for later.' },
]

export const STATUS_INFO: Record<ChangeStatus, { tone: 'plain' | 'info' | 'orange'; text: string; why?: string }> = {
  up_to_date: { tone: 'plain', text: 'Up to date' },
  clean: { tone: 'info', text: 'Ready to update' },
  dirty: { tone: 'orange', text: 'Needs attention', why: 'You have changed files here. Kora leaves this resource alone so your changes are not lost. To update it, put your changes aside first (for example with "git stash" in the resource folder; "git stash pop" brings them back), then check again. Committing them does not help: Kora never mixes your own commits with new ones.' },
  diverged: { tone: 'orange', text: 'Needs attention', why: 'This resource has commits of your own that the new version does not include (or Kora could not check it). Kora will not merge or drop them for you. Move your commits elsewhere or merge by hand, then check again.' },
}

/** Commit ids are long: the first 7 characters are enough to tell them apart. */
export const shortSha = (c: string) => (/^[0-9a-f]{8,}$/i.test(c) ? c.slice(0, 7) : c)

/** Did the run leave anything for a person (rolled back, needs attention, not attempted, held back)? */
export function reportHasProblems(r: UpdateReport): boolean {
  return r.rolled_back.length > 0 || r.needs_attention.length > 0 || (r.not_attempted?.length ?? 0) > 0 || (r.held?.length ?? 0) > 0
}

/** Put the freshly checked rows into the list; every other row stays exactly as it was. A row that is not in the list yet is added. */
export function mergeRows(view: UpdatePreview, fresh: ResourceChange[]): UpdatePreview {
  const byId = new Map(fresh.map((r) => [r.id, r]))
  const known = new Set(view.resources.map((r) => r.id))
  return { ...view, resources: [...view.resources.map((r) => byId.get(r.id) ?? r), ...fresh.filter((r) => !known.has(r.id))] }
}

function ChangeRow({ r, selected, onToggle, onCheck, checking, disabled, checkedAt, now }: { r: ResourceChange; selected: boolean; onToggle: () => void; onCheck: () => void; checking: boolean; disabled: boolean; checkedAt?: number; now: number }) {
  const info = STATUS_INFO[r.status]
  const selectable = r.status === 'clean'
  return (
    <li className="space-y-2 py-3">
      <div className="flex items-start gap-3">
        <input type="checkbox" aria-label={`Update ${r.id}`} className="mt-1 size-4 accent-lime" checked={selected} disabled={!selectable} onChange={onToggle} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-mono text-sm">{r.id}</span>
            <Badge tone={info.tone}>{info.text}</Badge>
            {r.needs_restart && selectable && <Badge tone="amber">Needs a restart</Badge>}
            {checkedAt !== undefined && <span className="text-xs text-muted-foreground">Last checked {timeAgo(checkedAt, now)}</span>}
          </div>
          {r.status !== 'up_to_date' && <div className="mt-1 text-xs text-muted-foreground"><code className="font-mono">{shortSha(r.from)}</code> to <code className="font-mono">{shortSha(r.to)}</code>{r.behind ? `, ${r.behind} new change${r.behind === 1 ? '' : 's'}` : ''}</div>}
          {info.why && <p className="mt-1 text-xs text-orange">{info.why}</p>}
          {r.changelog.length > 0 && (
            <details className="mt-1 text-xs text-muted-foreground">
              <summary className="cursor-pointer">What changed</summary>
              <ul className="mt-1 list-disc space-y-0.5 pl-5">{r.changelog.map((c, i) => <li key={i}>{c}</li>)}</ul>
            </details>
          )}
        </div>
        <Button size="sm" variant="ghost" disabled={disabled} onClick={onCheck} aria-label={`Check ${r.id}`} aria-busy={checking} title="Check only this resource for updates">
          {checking ? <Loader2 className="animate-spin" aria-hidden /> : <RefreshCw aria-hidden />}
          Check
        </Button>
      </div>
    </li>
  )
}

export function ReportView({ r }: { r: UpdateReport }) {
  const sec = (title: string, items: string[]) => items.length > 0 && <div><div className="text-sm font-semibold">{title}</div><ul className="list-disc pl-5 text-sm text-muted-foreground">{items.map((i, k) => <li key={k}>{i}</li>)}</ul></div>
  return (
    <div className="space-y-3">
      {sec('Updated', r.updated.map((u) => `${u.id} (${shortSha(u.from)} to ${shortSha(u.to)})`))}
      {sec('Skipped', r.skipped.map(([i, t]) => `${i}: ${t}`))}
      {sec('Restart left for later', r.deferred.map(([i, t]) => `${i}: ${t}`))}
      {sec('Rolled back automatically', r.rolled_back.map(([i, t]) => `${i}: ${t}`))}
      {sec('Contains database changes (check these)', r.sql_flagged)}
      {sec('Needs your attention', r.needs_attention.map(([i, t]) => `${i}: ${t}`))}
      {sec('Not attempted (the update stopped before it got here; nothing was changed)', r.not_attempted ?? [])}
      {sec('Held back', (r.held ?? []).map(([i, t]) => `${i}: ${t}`))}
      {sec('Notes', r.notes ?? [])}
      {sec('Forks brought up to date on GitHub', r.forks_synced)}
      {sec('Forks left alone', r.forks_left_alone.map(([i, t]) => `${i}: ${t}`))}
    </div>
  )
}

/** The whole-server update flow: check, pick, apply, progress, snapshots and rollback. Shown at the top of the Resources tab. */
export function UpdatesPanel({ server: s }: { server: ServerDetail }) {
  const qc = useQueryClient()
  const [checkJob, setCheckJob] = useState<string | null>(null)
  const [applyJob, setApplyJob] = useState<string | null>(null)
  const [rollJob, setRollJob] = useState<string | null>(null)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [restart, setRestart] = useState<RestartPolicy>('when_quiet')
  const [err, setErr] = useState<unknown>(null)
  const [rollbackId, setRollbackId] = useState<string | null>(null)
  // The snapshot of the last rollback, and whether "roll back anyway" was asked for.
  const [lastRollback, setLastRollback] = useState<string | null>(null)
  const [force, setForce] = useState(false)

  const [view, setView] = useState<UpdatePreview | null>(null)
  const [rowJob, setRowJob] = useState<{ id: string; job: string } | null>(null)
  const [checkedAt, setCheckedAt] = useState<Record<string, number>>({})
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30_000)
    return () => clearInterval(t)
  }, [])

  const check = useJob<UpdatePreview>(checkJob, (j) => {
    if (j.state === 'done' && j.result) {
      const res = j.result
      setView(res)
      setSelected(new Set(res.resources.filter((r) => r.status === 'clean').map((r) => r.id)))
      setCheckedAt(Object.fromEntries(res.resources.map((r) => [r.id, Date.now()])))
      setNow(Date.now())
    }
    void qc.invalidateQueries({ queryKey: ['servers'] })
    void qc.invalidateQueries({ queryKey: ['server', s.name] })
  })
  // One resource: only that row changes; the selection of the other rows is kept.
  const rowCheck = useJob<UpdatePreview>(rowJob?.job ?? null, (j) => {
    if (j.state !== 'done' || !j.result) return
    const res = j.result
    setView((v) => (v ? mergeRows(v, res.resources) : v))
    setSelected((x) => {
      const n = new Set(x)
      for (const r of res.resources) if (r.status !== 'clean') n.delete(r.id)
      return n
    })
    setCheckedAt((c) => ({ ...c, ...Object.fromEntries(res.resources.map((r) => [r.id, Date.now()])) }))
    setNow(Date.now())
    void qc.invalidateQueries({ queryKey: ['server', s.name] })
  })
  const apply = useJob<UpdateReport>(applyJob, () => {
    void qc.invalidateQueries({ queryKey: ['snapshots', s.name] })
    void qc.invalidateQueries({ queryKey: ['servers'] })
    void qc.invalidateQueries({ queryKey: ['server', s.name] })
  })
  const roll = useJob(rollJob, () => void qc.invalidateQueries({ queryKey: ['server', s.name] }))
  const snaps = useQuery({ queryKey: ['snapshots', s.name], queryFn: () => api.snapshots.list(s.name) })

  const preview = view
  const clean = useMemo(() => (preview?.resources ?? []).filter((r) => r.status === 'clean'), [preview])
  const checkingRow = rowCheck.phase === 'running' ? rowJob?.id : undefined
  const busy = check.phase === 'running' || rowCheck.phase === 'running' || apply.phase === 'running' || roll.phase === 'running'
  const report = apply.result
  const applyEvents = apply.events.map(normaliseUpdateEvent)
  const snapshotId = applyEvents.find((e) => e.type === 'snapshot')?.id
  const health = [...applyEvents].reverse().find((e) => e.type === 'health')?.message

  const toggle = (id: string) => setSelected((x) => { const n = new Set(x); if (n.has(id)) n.delete(id); else n.add(id); return n })

  const doCheck = async () => {
    setErr(null)
    setApplyJob(null)
    try { setCheckJob((await api.update.preview(s.name)).job) } catch (e) { setErr(e) }
  }
  const doCheckRow = async (id: string) => {
    setErr(null)
    try { setRowJob({ id, job: (await api.update.preview(s.name, { resources: [id] })).job }) } catch (e) { setErr(e) }
  }
  const doApply = async () => {
    setErr(null)
    const skip = (preview?.resources ?? []).filter((r) => !selected.has(r.id)).map((r) => r.id)
    // Exactly the versions shown here, even if something newer was published since the check.
    const targets = Object.fromEntries(clean.filter((r) => selected.has(r.id)).map((r) => [r.id, r.to]))
    try { setApplyJob((await api.update.apply(s.name, { scope: { resources: [...selected] }, restart, skip, targets })).job) } catch (e) { setErr(e) }
  }
  const doRollback = async () => {
    if (!rollbackId) return
    setErr(null)
    setLastRollback(rollbackId)
    try { setRollJob((await api.snapshots.rollback(s.name, rollbackId, force)).job) } catch (e) { setErr(e) }
    setRollbackId(null)
    setForce(false)
  }
  // A rollback that stopped because of changes of the person's own can be repeated, throwing them away.
  const rollError = roll.phase === 'failed' ? String((roll.error as { message?: string } | null)?.message ?? roll.error ?? '') : ''
  const blockedByChanges = rollError.includes('changed files of your own')

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader><CardTitle>Updates</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center gap-3">
            <Button variant={preview ? 'secondary' : 'default'} disabled={busy} onClick={doCheck}>{check.phase === 'running' ? 'Checking...' : 'Check for updates'}</Button>
            <span className="text-sm text-muted-foreground">Looks for newer versions. Nothing is changed yet.</span>
          </div>
          <ErrorNotice error={err ?? (check.phase === 'failed' ? check.error : rowCheck.phase === 'failed' ? rowCheck.error : null)} />
          {preview && (
            <>
              {(preview.notes ?? []).length > 0 && <Alert tone="warning" title="Please note"><ul className="list-disc pl-5">{preview.notes!.map((n, i) => <li key={i}>{n}</li>)}</ul></Alert>}
              {preview.new_release && <Alert tone="info">A new Kora release is available: {preview.new_release}.</Alert>}
              {preview.resources.length === 0 && <p className="text-sm text-muted-foreground">No resources were found to check.</p>}
              {preview.resources.length > 0 && clean.length === 0 && !preview.resources.some((r) => r.status === 'dirty' || r.status === 'diverged') && <Alert tone="success">Everything is up to date.</Alert>}
              <ul className="divide-y divide-border">
                {preview.resources.map((r) => <ChangeRow key={r.id} r={r} selected={selected.has(r.id)} onToggle={() => toggle(r.id)} onCheck={() => void doCheckRow(r.id)} checking={checkingRow === r.id} disabled={busy} checkedAt={checkedAt[r.id]} now={now} />)}
              </ul>
              {clean.length > 0 && (
                <div className="space-y-4 border-t border-border pt-4">
                  <div className="flex flex-wrap gap-2">
                    <Button size="sm" onClick={() => setSelected(new Set(clean.map((r) => r.id)))}>Select all</Button>
                    <Button size="sm" onClick={() => setSelected(new Set())}>Select none</Button>
                  </div>
                  <fieldset className="space-y-2">
                    <legend className="mb-1 text-sm font-semibold">After updating</legend>
                    {RESTART_OPTIONS.map((o) => <Radio key={o.value} name="restart" value={o.value} checked={restart === o.value} onChange={() => setRestart(o.value)} label={o.label} hint={o.hint} />)}
                  </fieldset>
                  <p className="text-xs text-muted-foreground">Kora saves a safety copy first, so you can roll back. If Kora restarts the server and it does not start again, Kora puts the earlier versions back by itself. It does not check whether each resource works once the server is running.</p>
                  <Button variant="default" disabled={busy || selected.size === 0} onClick={doApply}>Update {selected.size} resource{selected.size === 1 ? '' : 's'}</Button>
                </div>
              )}
            </>
          )}
        </CardContent>
      </Card>

      {apply.phase !== 'idle' && (
        <Card>
          <CardHeader><CardTitle>Update progress</CardTitle></CardHeader>
          <CardContent className="space-y-3">
            <ResourceProgress events={apply.events} />
            {snapshotId && <p className="text-sm">Safety copy: <code className="font-mono text-xs">{snapshotId}</code></p>}
            {health && <p className="text-sm">Health check: {health}</p>}
            {apply.phase === 'failed' && <ErrorNotice error={apply.error} title="The update did not finish" />}
            {apply.phase === 'done' && report && (
              <>
                {reportHasProblems(report)
                  ? <Alert tone="warning" title="Update finished, but some things need a look">See the details below.</Alert>
                  : <Alert tone="success" title="Update finished" />}
                <ReportView r={report} />
              </>
            )}
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader><CardTitle>Snapshots</CardTitle></CardHeader>
        <CardContent className="space-y-3">
          <p className="text-sm text-muted-foreground">Every update saves a snapshot first. Rolling back puts the resources in it back to how they were.</p>
          {snaps.data?.length === 0 && <p className="text-sm text-muted-foreground">No snapshots yet.</p>}
          <ul className="divide-y divide-border">
            {(snaps.data ?? []).map((sn) => (
              <li key={sn.id} className="flex flex-wrap items-center justify-between gap-3 py-3 text-sm">
                <div>
                  <div className="font-mono text-xs">{sn.id}</div>
                  <div className="text-muted-foreground">{formatDate(sn.created_unix)}: {sn.resources.join(', ')}</div>
                </div>
                <Button size="sm" disabled={busy} onClick={() => setRollbackId(sn.id)} aria-label={`Roll back to ${sn.id}`}>Roll back</Button>
              </li>
            ))}
          </ul>
          {roll.phase !== 'idle' && <ResourceProgress events={roll.events} />}
          {roll.phase === 'done' && <Alert tone="success">Rolled back.</Alert>}
          {roll.phase === 'failed' && <ErrorNotice error={roll.error} />}
          {roll.phase === 'failed' && blockedByChanges && lastRollback && (
            <Button size="sm" variant="destructive" disabled={busy} onClick={() => { setForce(true); setRollbackId(lastRollback) }}>Roll back anyway</Button>
          )}
        </CardContent>
      </Card>
      <ConfirmDialog
        open={!!rollbackId}
        onOpenChange={(o) => { if (!o) { setRollbackId(null); setForce(false) } }}
        title={force ? 'Roll back and throw away your changes?' : 'Roll back to this snapshot?'}
        description={force
          ? 'The resources in this snapshot go back to their earlier versions. Files you changed in them are thrown away and cannot be brought back. The server restarts if it is running.'
          : 'The resources in this snapshot go back to their earlier versions, and the server restarts if it is running. If a resource has files you changed, nothing is rolled back and Kora tells you which ones.'}
        confirmLabel={force ? 'Roll back anyway' : 'Roll back'}
        destructive
        onConfirm={doRollback}
      />
    </div>
  )
}
