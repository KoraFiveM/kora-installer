import { useEffect, useState } from 'react'
import { Loader2, RefreshCw } from 'lucide-react'
import { useQuery } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ChangeStatus, RestartPolicy, ResourceChange, ServerDetail, UpdateReport } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
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
import type { Updates } from './useUpdates'

export const RESTART_OPTIONS: { value: RestartPolicy; label: string; hint: string }[] = [
  { value: 'none', label: 'Do not restart', hint: 'Update the files only. The changes load the next time the server restarts.' },
  { value: 'ensure', label: 'Update files, reload later', hint: 'Update the files without restarting. Resources reload when someone runs "ensure" in the txAdmin console, or at the next restart.' },
  { value: 'full_restart', label: 'Restart the whole server', hint: 'Always restart after updating. Players are disconnected for a minute or so.' },
  { value: 'when_quiet', label: 'Restart only when nobody is playing', hint: 'If players are online, the restart is left for later.' },
]

export const STATUS_INFO: Record<ChangeStatus, { tone: 'plain' | 'info' | 'orange'; text: string; why?: string }> = {
  up_to_date: { tone: 'plain', text: 'Up to date' },
  clean: { tone: 'info', text: 'Ready to update' },
  dirty: { tone: 'orange', text: 'Needs attention', why: 'You changed files in this resource, so Kora left it alone and did not update it. Your changes are untouched. What to do: open the resource folder in a terminal and run "git stash" to set your changes aside, then check again. After the update, "git stash pop" brings them back. Saving them as a commit does not help: Kora never mixes your own commits with new ones.' },
  diverged: { tone: 'orange', text: 'Needs attention', why: 'This resource has saved changes of your own (commits) that the new version does not include, or Kora could not check it. Kora did not update it. It will not merge your commits or drop them for you. What to do: move your commits somewhere else or merge the new version by hand with git, then check again.' },
}

/** Commit ids are long: the first 7 characters are enough to tell them apart. */
export const shortSha = (c: string) => (/^[0-9a-f]{8,}$/i.test(c) ? c.slice(0, 7) : c)

/** Did the run leave anything for a person (rolled back, needs attention, not attempted, held back)? */
export function reportHasProblems(r: UpdateReport): boolean {
  return r.rolled_back.length > 0 || r.needs_attention.length > 0 || (r.not_attempted?.length ?? 0) > 0 || (r.held?.length ?? 0) > 0
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
export function UpdatesPanel({ server: s, updates: u }: { server: ServerDetail; updates: Updates }) {
  const role = useRole()
  const noOperator = whyNot(role, 'operator')
  const noAdmin = whyNot(role, 'admin')
  // The snapshot the dialog is asking about, and whether "roll back anyway" was asked for.
  const [rollbackId, setRollbackId] = useState<string | null>(null)
  const [force, setForce] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30_000)
    return () => clearInterval(t)
  }, [])
  const snaps = useQuery({ queryKey: ['snapshots', s.name], queryFn: () => api.snapshots.list(s.name) })

  const { view, apply, roll, busy } = u
  // Only what needs a decision is listed here; every resource and its state is in the list below.
  const pending = u.rows.filter((r) => r.status !== 'up_to_date')
  const upToDate = u.rows.length - pending.length
  const report = apply.result
  const applyEvents = apply.events.map(normaliseUpdateEvent)
  const snapshotId = applyEvents.find((e) => e.type === 'snapshot')?.id
  const health = [...applyEvents].reverse().find((e) => e.type === 'health')?.message
  const checking = u.check.phase === 'running'

  // A rollback that stopped because of changes of the person's own can be repeated, throwing them away.
  const rollError = roll.phase === 'failed' ? String((roll.error as { message?: string } | null)?.message ?? roll.error ?? '') : ''
  const blockedByChanges = rollError.includes('changed files of your own')

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader><CardTitle>Updates</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap items-center gap-3">
            <Button variant={view ? 'secondary' : 'default'} disabled={busy || noOperator !== null} title={noOperator ?? undefined} onClick={() => void u.checkAll()} aria-busy={checking}>
              {checking && <Loader2 className="animate-spin" aria-hidden />}
              {checking ? 'Checking...' : view ? 'Check again' : 'Check for updates'}
            </Button>
            <span className="text-sm text-muted-foreground" role="status">
              {checking ? 'Looking at every resource. This can take a minute; you can open other tabs meanwhile.' : noOperator ?? 'Looks for newer versions. Nothing is changed yet.'}
            </span>
          </div>
          <ErrorNotice error={u.error} />
          {view && (
            <>
              {(view.notes ?? []).length > 0 && <Alert tone="warning" title="Please note"><ul className="list-disc pl-5">{view.notes!.map((n, i) => <li key={i}>{n}</li>)}</ul></Alert>}
              {view.new_release && <Alert tone="info">A new Kora release is available: {view.new_release}.</Alert>}
              {u.full && u.rows.length === 0 && <p className="text-sm text-muted-foreground">No resources were found to check.</p>}
              {u.full && u.rows.length > 0 && pending.length === 0 && <Alert tone="success">Everything is up to date.</Alert>}
              {pending.length > 0 && (
                <>
                  <ul className="divide-y divide-border" aria-label="Resources with updates">
                    {pending.map((r) => <ChangeRow key={r.id} r={r} selected={u.selected.has(r.id)} onToggle={() => u.toggle(r.id)} onCheck={() => void u.checkOne(r.id)} checking={u.checkingRow === r.id} disabled={busy || noOperator !== null} checkedAt={u.checkedAt[r.id]} now={now} />)}
                  </ul>
                  {upToDate > 0 && <p className="text-xs text-muted-foreground">{upToDate} other resource{upToDate === 1 ? ' is' : 's are'} up to date.</p>}
                </>
              )}
              {u.clean.length > 0 && (
                <div className="space-y-4 border-t border-border pt-4">
                  {u.clean.length > 1 && (
                    <div className="flex flex-wrap gap-2">
                      <Button size="sm" onClick={u.selectAll}>Select all</Button>
                      <Button size="sm" onClick={u.selectNone}>Select none</Button>
                    </div>
                  )}
                  <fieldset className="space-y-2">
                    <legend className="mb-1 text-sm font-semibold">After updating</legend>
                    {RESTART_OPTIONS.map((o) => <Radio key={o.value} name="restart" value={o.value} checked={u.restart === o.value} onChange={() => u.setRestart(o.value)} label={o.label} hint={o.hint} />)}
                  </fieldset>
                  <p className="text-xs text-muted-foreground">Kora saves a safety copy first, so you can roll back. If Kora restarts the server and it does not start again, Kora puts the earlier versions back by itself. It does not check whether each resource works once the server is running.</p>
                  <Button variant="default" disabled={busy || u.chosen.length === 0 || noOperator !== null} title={noOperator ?? undefined} onClick={() => void u.applySelected()}>Update {u.chosen.length} resource{u.chosen.length === 1 ? '' : 's'}</Button>
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
          <ErrorNotice error={snaps.error} />
          {snaps.data?.length === 0 && <p className="text-sm text-muted-foreground">No snapshots yet.</p>}
          {(snaps.data?.length ?? 0) > 0 && noAdmin && <p className="text-xs text-muted-foreground">{noAdmin}</p>}
          <ul className="divide-y divide-border">
            {(snaps.data ?? []).map((sn) => (
              <li key={sn.id} className="flex flex-wrap items-center justify-between gap-3 py-3 text-sm">
                <div className="min-w-0">
                  <div className="font-mono text-xs">{sn.id}</div>
                  <div className="break-words text-muted-foreground">{formatDate(sn.created_unix)}: {sn.resources.join(', ')}</div>
                </div>
                <Button size="sm" disabled={busy || noAdmin !== null} title={noAdmin ?? undefined} onClick={() => setRollbackId(sn.id)} aria-label={`Roll back to ${sn.id}`}>Roll back</Button>
              </li>
            ))}
          </ul>
          {roll.phase !== 'idle' && <ResourceProgress events={roll.events} />}
          {roll.phase === 'done' && <Alert tone="success">Rolled back. Check for updates again to see where things stand.</Alert>}
          {roll.phase === 'failed' && <ErrorNotice error={roll.error} />}
          {roll.phase === 'failed' && blockedByChanges && u.lastRollback && (
            <Button size="sm" variant="destructive" disabled={busy || noAdmin !== null} onClick={() => { setForce(true); setRollbackId(u.lastRollback) }}>Roll back anyway</Button>
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
        onConfirm={() => {
          if (rollbackId) void u.rollback(rollbackId, force)
          setRollbackId(null)
          setForce(false)
        }}
      />
    </div>
  )
}
