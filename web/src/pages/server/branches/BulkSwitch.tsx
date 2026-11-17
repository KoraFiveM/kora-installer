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

interface CountEvent { type: string; done: number; total: number; id: string }

const STATUS_TEXT: Record<BulkStatus, { text: string; tone: 'success' | 'plain' | 'amber' | 'danger' }> = {
  switched: { text: 'Switched', tone: 'success' },
  already: { text: 'Already there', tone: 'plain' },
  needs_choice: { text: 'Has changes of yours', tone: 'amber' },
  skipped: { text: 'Left as it is', tone: 'plain' },
  failed: { text: 'Could not switch', tone: 'danger' },
}

/** Every resource at once: which ones have a branch, switch them, or put them all back to the normal version. */
export function BulkSwitch({ server: s }: { server: ServerDetail }) {
  const role = useRole()
  const blocked = whyNot(role, 'admin', true)
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
      <p className="text-sm text-muted-foreground">Working on several resources at once? Name a branch and Kora shows which resources have it before it changes anything. Resources without it are left alone.</p>
      <form
        className="flex flex-wrap items-end gap-2"
        onSubmit={async (e) => {
          e.preventDefault()
          setErr(null)
          setApplyJob(null)
          try { setPreviewJob((await api.branches.bulkPreview(s.name, branch.trim())).job) } catch (x) { setErr(x) }
        }}
      >
        <div className="min-w-56 flex-1"><Field label="Branch name"><Input value={branch} onChange={(e) => setBranch(e.target.value)} placeholder="For example: feature/new-ui" autoComplete="off" /></Field></div>
        <Button type="submit" disabled={!branch.trim() || checking || applying || noOperator !== null} title={noOperator ?? undefined}>{checking && <Loader2 className="animate-spin" aria-hidden />}Check which resources have it</Button>
      </form>
      {checking && <CountBar label="Checking resources" done={checked?.done ?? 0} total={checked?.total ?? 1} text={checked?.total ? `${checked.done} of ${checked.total} checked` : 'Asking GitHub...'} detail={checked?.id ? `Checked ${checked.id}` : undefined} />}
      <ErrorNotice error={err ?? (preview.phase === 'failed' ? preview.error : null) ?? (apply.phase === 'failed' ? apply.error : null)} />

      {p && (
        <div className="space-y-3">
          <Alert tone={p.have > 0 ? 'info' : 'warning'} title={`${p.have} of ${p.have + p.missing} resources have ${p.branch}`}>
            {p.have === 0 ? 'None of them has this branch, so nothing can be switched. Check the spelling.' : `${p.missing} do not have it and would stay as they are.`}
            {p.notes.map((n) => <p key={n}>{n}</p>)}
          </Alert>
          <ul className="divide-y divide-border rounded-md border border-border" aria-label="Which resources have the branch">
            {p.rows.map((r) => (
              <li key={r.id} className="flex flex-wrap items-center gap-2 p-2.5 text-sm">
                <span className="min-w-0 flex-1 break-all font-mono text-xs">{r.id}</span>
                {r.note ? <Badge tone="orange">Could not check</Badge> : r.already ? <Badge tone="plain">Already on it</Badge> : r.has ? <Badge tone="success">Has it</Badge> : <Badge tone="plain">Does not have it</Badge>}
                <span className="text-xs text-muted-foreground">now: {r.now}</span>
                {r.changed_count > 0 && <Badge tone="amber">{r.changed_count} changed file{r.changed_count === 1 ? '' : 's'}</Badge>}
                {r.note && <span className="w-full text-xs text-muted-foreground">{r.note}</span>}
              </li>
            ))}
          </ul>
          {ready.length > 0 && (
            <div>
              <Button variant="default" disabled={applying || blocked !== null} title={blocked ?? undefined} onClick={() => void start({ action: 'branch', branch: p.branch, resources: ready.map((r) => r.id) })}>
                Switch the {ready.length} that {ready.length === 1 ? 'has' : 'have'} it
              </Button>
              {blocked && <p className="mt-1 text-xs text-muted-foreground">{blocked}</p>}
              <p className="mt-1 text-xs text-muted-foreground">If one has changes of your own, Kora asks what to do before touching it.</p>
            </div>
          )}
        </div>
      )}

      {applying && <CountBar label="Switching resources" done={switchedEvent?.done ?? 0} total={switchedEvent?.total ?? 1} text={switchedEvent?.total ? `${switchedEvent.done} of ${switchedEvent.total} done` : 'Starting...'} detail={switchedEvent?.id ? `Done ${switchedEvent.id}` : undefined} />}
      {apply.phase === 'done' && apply.result && (
        <div className="space-y-3">
          <Alert tone={apply.result.failed > 0 ? 'warning' : 'success'} title={`${apply.result.switched} switched${apply.result.needs_choice ? `, ${apply.result.needs_choice} waiting for your choice` : ''}${apply.result.failed ? `, ${apply.result.failed} could not be switched` : ''}`}>
            {apply.result.restart_note && <p>{apply.result.restart_note} A running server still has the old versions until it restarts: restart it from the Overview tab.</p>}
          </Alert>
          <ul className="divide-y divide-border rounded-md border border-border" aria-label="What happened to each resource">
            {apply.result.items.map((i) => (
              <li key={i.id} className="space-y-1 p-2.5 text-sm">
                <div className="flex flex-wrap items-center gap-2"><span className="min-w-0 flex-1 break-all font-mono text-xs">{i.id}</span><Badge tone={STATUS_TEXT[i.status].tone}>{STATUS_TEXT[i.status].text}</Badge></div>
                <p className="text-xs text-muted-foreground">{i.message}</p>
                {i.stashed && <p className="text-xs text-muted-foreground">Your changes are kept aside; bring them back from this resource's Branch window.</p>}
              </li>
            ))}
          </ul>
          {waiting.length > 0 && <Button disabled={blocked !== null} onClick={() => setAsking(true)}>Choose what to do with the {waiting.length} that {waiting.length === 1 ? 'has' : 'have'} changes</Button>}
        </div>
      )}

      <div className="space-y-2 border-t border-border pt-4">
        <h3 className="text-sm font-semibold">Put everything back to the normal version</h3>
        <p className="text-xs text-muted-foreground">Every resource that is on another branch or a pull request goes back to its normal branch, so updates work for all of them again.</p>
        <Button disabled={applying || blocked !== null} title={blocked ?? undefined} onClick={() => setAskDefault(true)}>Back to the normal version, for all</Button>
      </div>

      <ConfirmDialog
        open={askDefault}
        onOpenChange={setAskDefault}
        title="Put every resource back to the normal version?"
        description="Resources on another branch or a pull request are switched back. If any has changes of your own, Kora asks first. Nothing is thrown away without you saying so."
        confirmLabel="Switch them all back"
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
