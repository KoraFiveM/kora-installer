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
import { CountBar, normaliseUpdateEvent, percentOf, ResourceProgress } from '@/components/JobProgress'
import { Announcer, progressSentence } from '@/components/Announcer'
import { Progress } from '@/components/ui/progress'
import { formatDate } from '@/lib/utils'
import { timeAgo } from '@/lib/ago'
import type { Updates } from './useUpdates'
import { applyProgress, checkProgress, rowCheckProgress, type CheckProgress } from './updateProgress'
import { t as tt, msg, type Msg } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/resources'

export const RESTART_OPTIONS: { value: RestartPolicy; label: Msg; hint: Msg }[] = [
  { value: 'none', label: msg('resources.updatesPanel.doRestart'), hint: msg('resources.updatesPanel.updateFilesOnlyChanges') },
  { value: 'ensure', label: msg('resources.updatesPanel.updateFilesReloadLater'), hint: msg('resources.updatesPanel.updateFilesWithoutRestarting') },
  { value: 'full_restart', label: msg('resources.updatesPanel.restartWholeServer'), hint: msg('resources.updatesPanel.alwaysRestartAfterUpdating') },
  { value: 'when_quiet', label: msg('resources.updatesPanel.restartOnlyWhenNobody'), hint: msg('resources.updatesPanel.playersOnlineRestartLeft') },
]

export const STATUS_INFO: Record<ChangeStatus, { tone: 'plain' | 'info' | 'amber' | 'orange'; text: Msg; why?: Msg }> = {
  up_to_date: { tone: 'plain', text: msg('resources.updatesPanel.upDate') },
  clean: { tone: 'info', text: msg('resources.updatesPanel.readyUpdate') },
  dirty: { tone: 'orange', text: msg('resources.updatesPanel.needsAttention2'), why: msg('resources.updatesPanel.changedFilesResourceKora') },
  other_branch: { tone: 'amber', text: msg('resources.updatesPanel.updatedAnotherBranch'), why: msg('resources.updatesPanel.putResourceAnotherBranch') },
  diverged: { tone: 'orange', text: msg('resources.updatesPanel.needsAttention2'), why: msg('resources.updatesPanel.resourceSavedChangesOwn') },
}

/** Commit ids are long: the first 7 characters are enough to tell them apart. */
export const shortSha = (c: string) => (/^[0-9a-f]{8,}$/i.test(c) ? c.slice(0, 7) : c)

/** Did the run leave anything for a person (rolled back, needs attention, not attempted, held back)? */
export function reportHasProblems(r: UpdateReport): boolean {
  return r.rolled_back.length > 0 || r.needs_attention.length > 0 || (r.not_attempted?.length ?? 0) > 0 || (r.held?.length ?? 0) > 0
}

/** One row. `progress` is given only to the row that is being checked on its own, so only that row shows a bar. */
export function ChangeRow({ r, selected, onToggle, onCheck, checking, disabled, checkedAt, now, progress }: { r: ResourceChange; selected: boolean; onToggle: () => void; onCheck: () => void; checking: boolean; disabled: boolean; checkedAt?: number; now: number; progress?: CheckProgress }) {
  const info = STATUS_INFO[r.status]
  const selectable = r.status === 'clean'
  return (
    <li className="space-y-2 py-3">
      <div className="flex items-start gap-3">
        <input type="checkbox" aria-label={tt('resources.updatesPanel.updateId', { id: r.id })} className="mt-1 size-4 accent-lime" checked={selected} disabled={!selectable} onChange={onToggle} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-mono text-sm">{r.id}</span>
            <Badge tone={info.tone}>{tt(info.text)}</Badge>
            {r.needs_restart && selectable && <Badge tone="amber">{tt('resources.updatesPanel.needsRestart')}</Badge>}
            {checkedAt !== undefined && <span className="text-xs text-muted-foreground">{tt('resources.updatesPanel.lastCheckedTimeago', { timeAgo: timeAgo(checkedAt, now) })}</span>}
          </div>
          {progress && (
            <div className="mt-2 flex items-center gap-2 text-xs text-muted-foreground">
              <Announcer message={progressSentence(`Checking ${r.id}`, percentOf(progress.done, progress.total))} />
              <Progress value={percentOf(progress.done, progress.total)} label={tt('resources.updatesPanel.checkingId', { id: r.id })} className="h-1 w-32 shrink-0" />
              <span>{progress.done > 0 ? tt('resources.updatesPanel.checkedId', { id: r.id }) : tt('resources.updatesPanel.checkingId2', { id: r.id })}</span>
            </div>
          )}
          {r.status !== 'up_to_date' && r.status !== 'other_branch' && <div className="mt-1 text-xs text-muted-foreground">{rich('resources.updatesPanel.codeNodeCodeNode', { code: <code className="font-mono">{shortSha(r.from)}</code>, code2: <code className="font-mono">{shortSha(r.to)}</code>, value: r.behind ? tt('resources.updatesPanel.behindNewChangeValue', { behind: r.behind }) : '' })}</div>}
          {r.status === 'other_branch'
            ? <p className="mt-1 text-xs text-muted-foreground">{tt('resources.updatesPanel.valueGoBackNormal', { value: r.note ?? tt('resources.updatesPanel.anotherBranch') })}</p>
            : info.why && <p className="mt-1 text-xs text-orange">{tt(info.why)}</p>}
          {r.changelog.length > 0 && (
            <details className="mt-1 text-xs text-muted-foreground">
              <summary className="cursor-pointer">{tt('resources.updatesPanel.whatChanged')}</summary>
              <ul className="mt-1 list-disc space-y-0.5 pl-5">{r.changelog.map((c, i) => <li key={i}>{c}</li>)}</ul>
            </details>
          )}
        </div>
        <Button size="sm" variant="ghost" disabled={disabled} onClick={onCheck} aria-label={tt('resources.updatesPanel.checkId', { id: r.id })} aria-busy={checking} title={tt('resources.updatesPanel.checkOnlyResourceUpdates')}>{rich('resources.updatesPanel.nodeNodeCheck', { node: checking ? <Loader2 className="animate-spin" aria-hidden /> : <RefreshCw aria-hidden /> })}</Button>
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
      {sec(tt('resources.updatesPanel.restartLeftLater'), r.deferred.map(([i, t]) => `${i}: ${t}`))}
      {sec(tt('resources.updatesPanel.rolledBackAutomatically'), r.rolled_back.map(([i, t]) => `${i}: ${t}`))}
      {sec(tt('resources.updatesPanel.containsDatabaseChangesCheck'), r.sql_flagged)}
      {sec(tt('resources.updatesPanel.needsAttention'), r.needs_attention.map(([i, t]) => `${i}: ${t}`))}
      {sec(tt('resources.updatesPanel.attemptedUpdateStoppedBefore'), r.not_attempted ?? [])}
      {sec(tt('resources.updatesPanel.heldBack'), (r.held ?? []).map(([i, t]) => `${i}: ${t}`))}
      {sec('Notes', r.notes ?? [])}
      {sec(tt('resources.updatesPanel.forksBroughtUpDate'), r.forks_synced)}
      {sec(tt('resources.updatesPanel.forksLeftAlone'), r.forks_left_alone.map(([i, t]) => `${i}: ${t}`))}
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
  // Progress of the whole-server check, and of the update (the one-resource check has its own row bar).
  const checked = checkProgress(u.check.events)
  const applying = applyProgress(apply.events)

  // A rollback that stopped because of changes of the person's own can be repeated, throwing them away.
  const rollError = roll.phase === 'failed' ? String((roll.error as { message?: string } | null)?.message ?? roll.error ?? '') : ''
  const blockedByChanges = rollError.includes('changed files of your own')

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader><CardTitle>{tt('resources.updatesPanel.updates')}</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap items-center gap-3">
            <Button variant={view ? 'secondary' : 'default'} disabled={busy || noOperator !== null} title={noOperator ?? undefined} onClick={() => void u.checkAll()} aria-busy={checking}>
              {checking && <Loader2 className="animate-spin" aria-hidden />}
              {checking ? tt('resources.updatesPanel.checking') : view ? tt('resources.updatesPanel.checkAgain') : tt('resources.updatesPanel.checkUpdates')}
            </Button>
            <span className="text-sm text-muted-foreground" role="status">
              {checking ? tt('resources.updatesPanel.lookingEveryResourceTake') : noOperator ?? tt('resources.updatesPanel.looksNewerVersionsNothing')}
            </span>
          </div>
          {checking && (
            <CountBar
              label={tt('resources.updatesPanel.checkingResourcesUpdates')}
              done={checked?.done ?? 0}
              total={checked?.total ?? u.rows.length}
              text={checked ? tt('resources.updatesPanel.doneTotalResourcesChecked', { done: checked.done, total: checked.total }) : tt('resources.updatesPanel.starting')}
              detail={checked?.id ? `Checked ${checked.id}` : undefined}
            />
          )}
          <ErrorNotice error={u.error} />
          {view && (
            <>
              {(view.notes ?? []).length > 0 && <Alert tone="warning" title={tt('resources.updatesPanel.pleaseNote')}><ul className="list-disc pl-5">{view.notes!.map((n, i) => <li key={i}>{n}</li>)}</ul></Alert>}
              {view.new_release && <Alert tone="info">{tt('resources.updatesPanel.newKoraReleaseAvailable', { new_release: view.new_release })}</Alert>}
              {u.full && u.rows.length === 0 && <p className="text-sm text-muted-foreground">{tt('resources.updatesPanel.noResourcesWereFound')}</p>}
              {u.full && u.rows.length > 0 && pending.length === 0 && <Alert tone="success">{tt('resources.updatesPanel.everythingUpDate')}</Alert>}
              {pending.length > 0 && (
                <>
                  <ul className="divide-y divide-border" aria-label={tt('resources.updatesPanel.resourcesUpdates')}>
                    {pending.map((r) => <ChangeRow key={r.id} r={r} selected={u.selected.has(r.id)} onToggle={() => u.toggle(r.id)} onCheck={() => void u.checkOne(r.id)} checking={u.checkingRow === r.id} disabled={busy || noOperator !== null} checkedAt={u.checkedAt[r.id]} now={now} progress={rowCheckProgress(r.id, u.checkingRow, u.rowCheck.events)} />)}
                  </ul>
                  {upToDate > 0 && <p className="text-xs text-muted-foreground">{tt('resources.updatesPanel.uptodateOtherResourceValue', { upToDate, value: upToDate === 1 ? ' is' : 's are' })}</p>}
                </>
              )}
              {u.clean.length > 0 && (
                <div className="space-y-4 border-t border-border pt-4">
                  {u.clean.length > 1 && (
                    <div className="flex flex-wrap gap-2">
                      <Button size="sm" onClick={u.selectAll}>{tt('resources.updatesPanel.selectAll')}</Button>
                      <Button size="sm" onClick={u.selectNone}>{tt('resources.updatesPanel.selectNone')}</Button>
                    </div>
                  )}
                  <fieldset className="space-y-2">
                    <legend className="mb-1 text-sm font-semibold">{tt('resources.updatesPanel.afterUpdating')}</legend>
                    {RESTART_OPTIONS.map((o) => <Radio key={o.value} name="restart" value={o.value} checked={u.restart === o.value} onChange={() => u.setRestart(o.value)} label={tt(o.label)} hint={tt(o.hint)} />)}
                  </fieldset>
                  <p className="text-xs text-muted-foreground">{tt('resources.updatesPanel.koraSavesSafetyCopy')}</p>
                  <Button variant="default" disabled={busy || u.chosen.length === 0 || noOperator !== null} title={noOperator ?? undefined} onClick={() => void u.applySelected()}>{tt('resources.updatesPanel.updateCountResourceValue', { count: u.chosen.length })}</Button>
                </div>
              )}
            </>
          )}
        </CardContent>
      </Card>

      {apply.phase !== 'idle' && (
        <Card>
          <CardHeader><CardTitle>{tt('resources.updatesPanel.updateProgress')}</CardTitle></CardHeader>
          <CardContent className="space-y-3">
            {applying && (
              <CountBar
                label={tt('resources.updatesPanel.updatingResources')}
                done={applying.done}
                total={applying.total}
                text={tt('resources.updatesPanel.doneTotalUpdatedValue', { done: applying.done, total: applying.total, value: applying.failed > 0 ? `, ${applying.failed} failed` : '' })}
                detail={applying.current}
              />
            )}
            <ResourceProgress events={apply.events} />
            {snapshotId && <p className="text-sm">{rich('resources.updatesPanel.safetyCopyCodeNode', { code: <code className="font-mono text-xs">{snapshotId}</code> })}</p>}
            {health && <p className="text-sm">{tt('resources.updatesPanel.healthCheckHealth', { health })}</p>}
            {apply.phase === 'failed' && <ErrorNotice error={apply.error} title={tt('resources.updatesPanel.updateDidFinish')} />}
            {apply.phase === 'done' && report && (
              <>
                {reportHasProblems(report)
                  ? <Alert tone="warning" title={tt('resources.updatesPanel.updateFinishedButSome')}>{tt('resources.updatesPanel.seeDetailsBelow')}</Alert>
                  : <Alert tone="success" title={tt('resources.updatesPanel.updateFinished')} />}
                <ReportView r={report} />
              </>
            )}
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader><CardTitle>{tt('resources.updatesPanel.snapshots')}</CardTitle></CardHeader>
        <CardContent className="space-y-3">
          <p className="text-sm text-muted-foreground">{tt('resources.updatesPanel.everyUpdateSavesSnapshot')}</p>
          <ErrorNotice error={snaps.error} />
          {snaps.data?.length === 0 && <p className="text-sm text-muted-foreground">{tt('resources.updatesPanel.noSnapshotsYet')}</p>}
          {(snaps.data?.length ?? 0) > 0 && noAdmin && <p className="text-xs text-muted-foreground">{noAdmin}</p>}
          <ul className="divide-y divide-border">
            {(snaps.data ?? []).map((sn) => (
              <li key={sn.id} className="flex flex-wrap items-center justify-between gap-3 py-3 text-sm">
                <div className="min-w-0">
                  <div className="font-mono text-xs">{sn.id}</div>
                  <div className="break-words text-muted-foreground">{formatDate(sn.created_unix)}: {sn.resources.join(', ')}</div>
                  {(sn.on_other_branch ?? []).length > 0 && (
                    <div className="mt-1 flex flex-wrap items-center gap-1">
                      {(sn.on_other_branch ?? []).map((o) => <Badge key={o.id} tone="amber" title={tt('resources.updatesPanel.itsNormalVersionWhen')}>{tt('resources.updatesPanel.idBranchBranch', { id: o.id, branch: o.branch })}</Badge>)}
                    </div>
                  )}
                </div>
                <Button size="sm" disabled={busy || noAdmin !== null} title={noAdmin ?? undefined} onClick={() => setRollbackId(sn.id)} aria-label={tt('resources.updatesPanel.rollBackId', { id: sn.id })}>{tt('resources.updatesPanel.rollBack')}</Button>
              </li>
            ))}
          </ul>
          {roll.phase !== 'idle' && <ResourceProgress events={roll.events} />}
          {roll.phase === 'done' && <Alert tone="success">{tt('resources.updatesPanel.rolledBackCheckUpdates')}</Alert>}
          {roll.phase === 'failed' && <ErrorNotice error={roll.error} />}
          {roll.phase === 'failed' && blockedByChanges && u.lastRollback && (
            <Button size="sm" variant="destructive" disabled={busy || noAdmin !== null} onClick={() => { setForce(true); setRollbackId(u.lastRollback) }}>{tt('resources.updatesPanel.rollBackAnyway')}</Button>
          )}
        </CardContent>
      </Card>
      <ConfirmDialog
        open={!!rollbackId}
        onOpenChange={(o) => { if (!o) { setRollbackId(null); setForce(false) } }}
        title={force ? tt('resources.updatesPanel.rollBackThrowAway') : tt('resources.updatesPanel.rollBackSnapshot')}
        description={force
          ? tt('resources.updatesPanel.resourcesSnapshotGoBack')
          : tt('resources.updatesPanel.resourcesSnapshotGoBack2')}
        confirmLabel={force ? tt('resources.updatesPanel.rollBackAnyway') : tt('resources.updatesPanel.rollBack')}
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
