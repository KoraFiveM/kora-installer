import { useState } from 'react'
import { Loader2 } from 'lucide-react'
import { api } from '@/api/client'
import type { BulkPreview, BulkResult, BulkStatus, ChangesChoice, ServerDetail } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { CountBar } from '@/components/JobProgress'
import { ChangesDialog } from './ChangesDialog'
import { useRefreshBranches } from './useBranches'
import { t, msg, type Msg } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/resources/bulkSwitch'

interface CountEvent { type: string; done: number; total: number; id: string }

const STATUS_TEXT: Record<BulkStatus, { text: Msg; tone: 'success' | 'plain' | 'amber' | 'danger' }> = {
  switched: { text: msg('resources.bulkSwitch.switched'), tone: 'success' },
  already: { text: msg('resources.bulkSwitch.alreadyThere'), tone: 'plain' },
  needs_choice: { text: msg('resources.bulkSwitch.changesYours'), tone: 'amber' },
  skipped: { text: msg('resources.bulkSwitch.leftAsItIs'), tone: 'plain' },
  failed: { text: msg('resources.bulkSwitch.couldSwitch'), tone: 'danger' },
}

/** Every resource at once: which ones have a branch, switch them, or put them all back to the normal version. */
export function BulkSwitch({ server: s }: { server: ServerDetail }) {
  const role = useRole()
  const blocked = whyNot(role, 'admin')
  const noOperator = whyNot(role, 'operator')
  const refresh = useRefreshBranches(s.name)
  const [branch, setBranch] = useState('')
  const [previewJob, setPreviewJob] = useState<string | null>(null)
  const [applyJob, setApplyJob] = useState<string | null>(null)
  const [err, setErr] = useState<unknown>(null)
  // What the current apply is about, so "keep my changes aside" can run it again for the resources that asked.
  const [last, setLast] = useState<{ action: 'branch' | 'default'; branch: string; resources?: string[] } | null>(null)
  const [askDefault, setAskDefault] = useState(false)
  const [asking, setAsking] = useState(false)

  const preview = useJob<BulkPreview, CountEvent>(previewJob)
  const apply = useJob<BulkResult, CountEvent>(applyJob, () => refresh())
  const checking = preview.phase === 'running'
  const applying = apply.phase === 'running'
  const p = preview.phase === 'done' ? preview.result : null
  const checked = [...preview.events].reverse().find((e) => e.type === 'checked')
  const switchedEvent = [...apply.events].reverse().find((e) => e.type === 'switched')
  const ready = p ? p.rows.filter((r) => r.has && !r.already) : []
  const waiting = apply.phase === 'done' && apply.result ? apply.result.items.filter((i) => i.status === 'needs_choice') : []

  const start = async (req: { action: 'branch'; branch: string; resources?: string[]; changes?: ChangesChoice } | { action: 'default'; resources?: string[]; changes?: ChangesChoice }) => {
    setErr(null)
    try {
      setLast({ action: req.action, branch: req.action === 'branch' ? req.branch : '', resources: req.resources })
      setApplyJob((await api.branches.bulkApply(s.name, req)).job)
    } catch (e) {
      setErr(e)
    }
  }

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">{t('resources.bulkSwitch.workingSeveralResourcesOnce')}</p>
      <form
        className="flex flex-wrap items-end gap-2"
        onSubmit={async (e) => {
          e.preventDefault()
          setErr(null)
          setApplyJob(null)
          try { setPreviewJob((await api.branches.bulkPreview(s.name, branch.trim())).job) } catch (x) { setErr(x) }
        }}
      >
        <div className="min-w-56 flex-1"><Field label={t('resources.bulkSwitch.branchName')}><Input value={branch} onChange={(e) => setBranch(e.target.value)} placeholder={t('resources.bulkSwitch.exampleFeatureNewui')} autoComplete="off" /></Field></div>
        <Button type="submit" disabled={!branch.trim() || checking || applying || noOperator !== null} title={noOperator ?? undefined}>{rich('resources.bulkSwitch.nodeNodeCheckWhich', { node: checking && <Loader2 className="animate-spin" aria-hidden /> })}</Button>
      </form>
      {checking && <CountBar label={t('resources.bulkSwitch.checkingResources')} done={checked?.done ?? 0} total={checked?.total ?? 1} text={checked?.total ? t('resources.bulkSwitch.doneTotalChecked', { done: checked.done, total: checked.total }) : t('resources.bulkSwitch.askingGithub')} detail={checked?.id ? `Checked ${checked.id}` : undefined} />}
      <ErrorNotice error={err ?? (preview.phase === 'failed' ? preview.error : null) ?? (apply.phase === 'failed' ? apply.error : null)} />

      {p && (
        <div className="space-y-3">
          <Alert tone={p.have > 0 ? 'info' : 'warning'} title={t('resources.bulkSwitch.valueResourcesBranch', { have: p.have, value: p.have + p.missing, branch: p.branch })}>
            {p.have === 0 ? t('resources.bulkSwitch.noneThemBranchNothing') : t('resources.bulkSwitch.missingDoWouldStay', { missing: p.missing })}
            {p.notes.map((n) => <p key={n}>{n}</p>)}
          </Alert>
          <ul className="divide-y divide-border rounded-md border border-border" aria-label={t('resources.bulkSwitch.whichResourcesBranch')}>
            {p.rows.map((r) => (
              <li key={r.id} className="flex flex-wrap items-center gap-2 p-2.5 text-sm">
                <span className="min-w-0 flex-1 break-all font-mono text-xs">{r.id}</span>
                {r.note ? <Badge tone="orange">{t('resources.bulkSwitch.couldCheck')}</Badge> : r.already ? <Badge tone="plain">{t('resources.bulkSwitch.alreadyOnIt')}</Badge> : r.has ? <Badge tone="success">{t('resources.bulkSwitch.hasIt')}</Badge> : <Badge tone="plain">{t('resources.bulkSwitch.doesNotHaveIt')}</Badge>}
                {r.now.startsWith('the normal version') ? <span className="text-xs text-muted-foreground">{t('resources.bulkSwitch.nowNow', { now: r.now })}</span> : <Badge tone="amber" title={t('resources.bulkSwitch.itsNormalVersion')}>{t('resources.bulkSwitch.nowNow', { now: r.now })}</Badge>}
                {r.changed_count > 0 && <Badge tone="amber">{t('resources.bulkSwitch.changedCountChangedFile', { changed_count: r.changed_count })}</Badge>}
                {r.note && <span className="w-full text-xs text-muted-foreground">{r.note}</span>}
              </li>
            ))}
          </ul>
          {ready.length > 0 && (
            <div>
              <Button variant="default" disabled={applying || blocked !== null} title={blocked ?? undefined} onClick={() => void start({ action: 'branch', branch: p.branch, resources: ready.map((r) => r.id) })}>{t('resources.bulkSwitch.switchCountValue', { count: ready.length })}</Button>
              {blocked && <p className="mt-1 text-xs text-muted-foreground">{blocked}</p>}
              <p className="mt-1 text-xs text-muted-foreground">{t('resources.bulkSwitch.oneChangesOwnKora')}</p>
            </div>
          )}
        </div>
      )}

      {applying && <CountBar label={t('resources.bulkSwitch.switchingResources')} done={switchedEvent?.done ?? 0} total={switchedEvent?.total ?? 1} text={switchedEvent?.total ? t('resources.bulkSwitch.doneTotalDone', { done: switchedEvent.done, total: switchedEvent.total }) : t('resources.bulkSwitch.starting')} detail={switchedEvent?.id ? `Done ${switchedEvent.id}` : undefined} />}
      {apply.phase === 'done' && apply.result && (
        <div className="space-y-3">
          <Alert tone={apply.result.failed > 0 ? 'warning' : 'success'} title={t('resources.bulkSwitch.switchedSwitchedValueValue', { switched: apply.result.switched, value: apply.result.needs_choice ? t('resources.bulkSwitch.needsChoiceWaitingChoice', { needs_choice: apply.result.needs_choice }) : '', value2: apply.result.failed ? t('resources.bulkSwitch.failedCouldSwitched', { failed: apply.result.failed }) : '' })}>
            {apply.result.restart_note && <p>{t('resources.bulkSwitch.restartNoteRunningServer', { restart_note: apply.result.restart_note })}</p>}
          </Alert>
          <ul className="divide-y divide-border rounded-md border border-border" aria-label={t('resources.bulkSwitch.whatHappenedEachResource')}>
            {apply.result.items.map((i) => (
              <li key={i.id} className="space-y-1 p-2.5 text-sm">
                <div className="flex flex-wrap items-center gap-2"><span className="min-w-0 flex-1 break-all font-mono text-xs">{i.id}</span><Badge tone={STATUS_TEXT[i.status].tone}>{t(STATUS_TEXT[i.status].text)}</Badge></div>
                <p className="text-xs text-muted-foreground">{i.message}</p>
                {i.stashed && <p className="text-xs text-muted-foreground">{t('resources.bulkSwitch.changesKeptAsideBring')}</p>}
              </li>
            ))}
          </ul>
          {waiting.length > 0 && <Button disabled={blocked !== null} onClick={() => setAsking(true)}>{t('resources.bulkSwitch.chooseWhatDoCount', { count: waiting.length })}</Button>}
        </div>
      )}

      <div className="space-y-2 border-t border-border pt-4">
        <h3 className="text-sm font-semibold">{t('resources.bulkSwitch.putEverythingBackNormal')}</h3>
        <p className="text-xs text-muted-foreground">{t('resources.bulkSwitch.everyResourceAnotherBranch')}</p>
        <Button disabled={applying || blocked !== null} title={blocked ?? undefined} onClick={() => setAskDefault(true)}>{t('resources.bulkSwitch.backNormalVersionAll')}</Button>
      </div>

      <ConfirmDialog
        open={askDefault}
        onOpenChange={setAskDefault}
        title={t('resources.bulkSwitch.putEveryResourceBack')}
        description={t('resources.bulkSwitch.resourcesAnotherBranchPull')}
        confirmLabel={t('resources.bulkSwitch.switchThemAllBack')}
        onConfirm={() => { setAskDefault(false); setPreviewJob(null); void start({ action: 'default' }) }}
      />
      <ChangesDialog
        open={asking}
        resources={waiting.map((w) => w.id)}
        files={waiting.flatMap((w) => (w.files ?? []).map((f) => `${w.id}: ${f}`))}
        busy={applying}
        onCancel={() => setAsking(false)}
        onChoose={(choice) => {
          setAsking(false)
          if (!last) return
          const only = waiting.map((w) => w.id)
          void start(last.action === 'default' ? { action: 'default', resources: only, changes: choice } : { action: 'branch', branch: last.branch, resources: only, changes: choice })
        }}
      />
    </div>
  )
}
