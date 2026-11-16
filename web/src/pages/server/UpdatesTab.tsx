import { useMemo, useState } from 'react'
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

export const RESTART_OPTIONS: { value: RestartPolicy; label: string; hint: string }[] = [
  { value: 'none', label: 'Do not restart', hint: 'Update the files only. The changes load the next time the server restarts.' },
  { value: 'ensure', label: 'Update files, reload later', hint: 'Update the files without restarting. Resources reload when someone runs "ensure" in the txAdmin console, or at the next restart.' },
  { value: 'full_restart', label: 'Restart the whole server', hint: 'Always restart after updating. Players are disconnected for a minute or so.' },
  { value: 'when_quiet', label: 'Restart only when nobody is playing', hint: 'If players are online, the restart is left for later.' },
]

const STATUS_INFO: Record<ChangeStatus, { tone: 'plain' | 'info' | 'orange'; text: string; why?: string }> = {
  up_to_date: { tone: 'plain', text: 'Up to date' },
  clean: { tone: 'info', text: 'Ready to update' },
  dirty: { tone: 'orange', text: 'Needs attention', why: 'You have changes here that are not saved as a commit. Updating could overwrite them, so Kora leaves this one alone. Save or undo your changes, then check again.' },
  diverged: { tone: 'orange', text: 'Needs attention', why: 'This resource has commits of your own and the original has new ones too (or Kora could not compare them). Kora will not merge them for you. Merge by hand, then check again.' },
}

function ChangeRow({ r, selected, onToggle }: { r: ResourceChange; selected: boolean; onToggle: () => void }) {
  const info = STATUS_INFO[r.status]
  const selectable = r.status === 'clean'
  return (
    <li className="space-y-2 py-3">
      <div className="flex items-start gap-3">
        <input type="checkbox" aria-label={`Update ${r.id}`} className="mt-1 size-4 accent-[#a8ff00]" checked={selected} disabled={!selectable} onChange={onToggle} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-mono text-sm">{r.id}</span>
            <Badge tone={info.tone}>{info.text}</Badge>
            {r.needs_restart && selectable && <Badge tone="amber">Needs a restart</Badge>}
          </div>
          {r.status !== 'up_to_date' && <div className="mt-1 text-xs text-muted-foreground">{r.from} to {r.to}{r.behind ? `, ${r.behind} new change${r.behind === 1 ? '' : 's'}` : ''}</div>}
          {info.why && <p className="mt-1 text-xs text-orange">{info.why}</p>}
          {r.changelog.length > 0 && (
            <details className="mt-1 text-xs text-muted-foreground">
              <summary className="cursor-pointer">What changed</summary>
              <ul className="mt-1 list-disc space-y-0.5 pl-5">{r.changelog.map((c, i) => <li key={i}>{c}</li>)}</ul>
            </details>
          )}
        </div>
      </div>
    </li>
  )
}

function ReportView({ r }: { r: UpdateReport }) {
  const sec = (title: string, items: string[]) => items.length > 0 && <div><div className="text-sm font-semibold">{title}</div><ul className="list-disc pl-5 text-sm text-muted-foreground">{items.map((i, k) => <li key={k}>{i}</li>)}</ul></div>
  return (
    <div className="space-y-3">
      {sec('Updated', r.updated.map((u) => `${u.id} (${u.from} to ${u.to})`))}
      {sec('Skipped', r.skipped.map(([i, t]) => `${i}: ${t}`))}
      {sec('Restart left for later', r.deferred.map(([i, t]) => `${i}: ${t}`))}
      {sec('Rolled back automatically', r.rolled_back.map(([i, t]) => `${i}: ${t}`))}
      {sec('Contains database changes (check these)', r.sql_flagged)}
      {sec('Needs your attention', r.needs_attention.map(([i, t]) => `${i}: ${t}`))}
      {sec('Forks brought up to date on GitHub', r.forks_synced)}
      {sec('Forks left alone', r.forks_left_alone.map(([i, t]) => `${i}: ${t}`))}
    </div>
  )
}

export function UpdatesTab({ server: s }: { server: ServerDetail }) {
  const qc = useQueryClient()
  const [checkJob, setCheckJob] = useState<string | null>(null)
  const [applyJob, setApplyJob] = useState<string | null>(null)
  const [rollJob, setRollJob] = useState<string | null>(null)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [restart, setRestart] = useState<RestartPolicy>('when_quiet')
  const [err, setErr] = useState<unknown>(null)
  const [rollbackId, setRollbackId] = useState<string | null>(null)

  const check = useJob<UpdatePreview>(checkJob, (j) => {
    if (j.state === 'done' && j.result) setSelected(new Set(j.result.resources.filter((r) => r.status === 'clean').map((r) => r.id)))
    void qc.invalidateQueries({ queryKey: ['servers'] })
    void qc.invalidateQueries({ queryKey: ['server', s.name] })
  })
  const apply = useJob<UpdateReport>(applyJob, () => {
    void qc.invalidateQueries({ queryKey: ['snapshots', s.name] })
    void qc.invalidateQueries({ queryKey: ['servers'] })
    void qc.invalidateQueries({ queryKey: ['server', s.name] })
  })
  const roll = useJob(rollJob, () => void qc.invalidateQueries({ queryKey: ['server', s.name] }))
  const snaps = useQuery({ queryKey: ['snapshots', s.name], queryFn: () => api.snapshots.list(s.name) })

  const preview = check.result
  const clean = useMemo(() => (preview?.resources ?? []).filter((r) => r.status === 'clean'), [preview])
  const busy = check.phase === 'running' || apply.phase === 'running' || roll.phase === 'running'
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
  const doApply = async () => {
    setErr(null)
    const skip = (preview?.resources ?? []).filter((r) => !selected.has(r.id)).map((r) => r.id)
    try { setApplyJob((await api.update.apply(s.name, { scope: { resources: [...selected] }, restart, skip })).job) } catch (e) { setErr(e) }
  }
  const doRollback = async () => {
    if (!rollbackId) return
    setErr(null)
    try { setRollJob((await api.snapshots.rollback(s.name, rollbackId)).job) } catch (e) { setErr(e) }
    setRollbackId(null)
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader><CardTitle>Updates</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center gap-3">
            <Button variant={preview ? 'secondary' : 'default'} disabled={busy} onClick={doCheck}>{check.phase === 'running' ? 'Checking...' : 'Check for updates'}</Button>
            <span className="text-sm text-muted-foreground">Looks for newer versions. Nothing is changed yet.</span>
          </div>
          <ErrorNotice error={err ?? (check.phase === 'failed' ? check.error : null)} />
          {preview && (
            <>
              {preview.new_release && <Alert tone="info">A new Kora release is available: {preview.new_release}.</Alert>}
              {preview.resources.length === 0 && <p className="text-sm text-muted-foreground">No resources were found to check.</p>}
              {preview.resources.length > 0 && clean.length === 0 && !preview.resources.some((r) => r.status === 'dirty' || r.status === 'diverged') && <Alert tone="success">Everything is up to date.</Alert>}
              <ul className="divide-y divide-border">
                {preview.resources.map((r) => <ChangeRow key={r.id} r={r} selected={selected.has(r.id)} onToggle={() => toggle(r.id)} />)}
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
                  <p className="text-xs text-muted-foreground">Kora saves a safety copy first, so you can roll back. If the health check fails, it rolls back by itself.</p>
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
                <Alert tone="success" title="Update finished" />
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
        </CardContent>
      </Card>
      <ConfirmDialog open={!!rollbackId} onOpenChange={(o) => !o && setRollbackId(null)} title="Roll back to this snapshot?" description="The resources in this snapshot go back to their earlier versions, and the server restarts if it is running." confirmLabel="Roll back" destructive onConfirm={doRollback} />
    </div>
  )
}
