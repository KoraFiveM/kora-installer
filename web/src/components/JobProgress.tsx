import { CheckCircle2, Circle, Loader2, MinusCircle, XCircle } from 'lucide-react'
import type { ProgressEvent, StepStatus } from '@/api/types'
import { Progress } from '@/components/ui/progress'
import { Announcer, progressSentence } from '@/components/Announcer'
import { cn } from '@/lib/utils'
import { t, msg, type Msg } from '@/i18n'
import '@/i18n/en/core'

const STATUS_TEXT: Record<StepStatus, Msg> = { pending: msg('core.jobProgress.waiting'), running: msg('core.jobProgress.inProgress'), done: msg('core.jobProgress.done'), skipped: msg('core.jobProgress.skipped'), failed: msg('core.jobProgress.failed') }

function Icon({ status }: { status: StepStatus }) {
  const c = 'size-4 shrink-0'
  if (status === 'done') return <CheckCircle2 className={cn(c, 'text-success')} aria-hidden />
  if (status === 'running') return <Loader2 className={cn(c, 'animate-spin text-info')} aria-hidden />
  if (status === 'failed') return <XCircle className={cn(c, 'text-danger-text')} aria-hidden />
  if (status === 'skipped') return <MinusCircle className={cn(c, 'text-steel')} aria-hidden />
  return <Circle className={cn(c, 'text-steel')} aria-hidden />
}

/** Collapse a stream of ProgressEvents into the latest state of each step, in first-seen order. */
export function latestSteps(events: ProgressEvent[]): ProgressEvent[] {
  const order: string[] = []
  const by = new Map<string, ProgressEvent>()
  for (const e of events) {
    if (!by.has(e.step)) order.push(e.step)
    by.set(e.step, normaliseStep(e))
  }
  return order.map((k) => by.get(k)!)
}

/** The backend may send Rust-style capitalised statuses; accept both. */
export function normaliseStep(e: ProgressEvent): ProgressEvent {
  return { ...e, status: String(e.status).toLowerCase() as StepStatus }
}

/** Step list with statuses and an overall percent bar (install, and anything else that sends ProgressEvents). */
export function StepProgress({ events, planned }: { events: ProgressEvent[]; planned?: { id: string; title: string }[] }) {
  const steps = latestSteps(events)
  const known = new Set(steps.map((s) => s.step))
  const rows: ProgressEvent[] = [...steps, ...(planned ?? []).filter((p) => !known.has(p.id)).map((p) => ({ step: p.id, title: p.title, status: 'pending' as StepStatus, detail: '', percent: 0 }))]
  const percent = events.length ? Math.max(...events.map((e) => e.percent)) : 0
  // One sentence that changes when a step starts, fails or the whole thing ends: not on every percent.
  const failed = rows.find((r) => r.status === 'failed')
  const running = rows.find((r) => r.status === 'running')
  const allDone = rows.length > 0 && rows.every((r) => r.status === 'done' || r.status === 'skipped')
  const said = failed ? `${failed.title}: ${t(STATUS_TEXT.failed)}` : allDone ? t('core.jobProgress.allStepsFinished') : running ? `${running.title}: ${t(STATUS_TEXT.running)}` : ''
  return (
    <div className="space-y-4">
      <Announcer message={said} />
      <div className="flex items-center gap-3">
        <Progress value={percent} label={t('core.jobProgress.overallProgress')} className="flex-1" />
        <span className="w-10 text-right text-sm tabular-nums text-muted-foreground">{percent}%</span>
      </div>
      <ol className="space-y-2">
        {rows.map((s) => (
          <li key={s.step} className="flex items-start gap-3 text-sm">
            <Icon status={s.status} />
            <div className="min-w-0 flex-1">
              <div className={cn(s.status === 'pending' && 'text-muted-foreground')}>
                {s.title} <span className="sr-only">: {t(STATUS_TEXT[s.status])}</span>
              </div>
              {s.detail && s.status !== 'done' && <div className="text-xs text-muted-foreground">{s.detail}</div>}
            </div>
          </li>
        ))}
      </ol>
    </div>
  )
}

/** Percent of a count, 0 to 100 (0 when there is nothing to count). */
export function percentOf(done: number, total: number): number {
  return total > 0 ? Math.round((Math.min(Math.max(done, 0), total) / total) * 100) : 0
}

/** A count bar: the bar, the "N of M" text, and an optional line about what is happening now. */
export function CountBar({ label, done, total, text, detail, className }: { label: string; done: number; total: number; text: string; detail?: string; className?: string }) {
  return (
    <div className={cn('space-y-1', className)}>
      <Announcer message={progressSentence(label, percentOf(done, total), total > 0 && done >= total)} />
      <div className="flex items-center gap-3">
        <Progress value={percentOf(done, total)} label={label} className="flex-1" />
        <span className="shrink-0 text-sm tabular-nums text-muted-foreground">{text}</span>
      </div>
      {detail && <div className="text-xs text-muted-foreground">{detail}</div>}
    </div>
  )
}

export interface ResourceEvent {
  type?: string
  id?: string
  state?: string
  message?: string
}

/** Tolerates both `{type:'resource', id, state}` and Rust's default `{"Resource": {id, state}}` shapes. */
export function normaliseUpdateEvent(e: unknown): ResourceEvent {
  if (typeof e === 'string') return { type: e.toLowerCase() }
  if (!e || typeof e !== 'object') return {}
  const o = e as Record<string, unknown>
  if (typeof o.type === 'string') return o as ResourceEvent
  const key = Object.keys(o)[0]
  if (!key) return {}
  const v = o[key]
  if (typeof v === 'string') return key === 'Snapshot' ? { type: 'snapshot', id: v } : { type: key.toLowerCase(), message: v }
  return { type: key.toLowerCase(), ...(v as object) }
}

/** Per-resource progress for update / add-resources jobs. */
export function ResourceProgress({ events }: { events: unknown[] }) {
  const evs = events.map(normaliseUpdateEvent)
  const rows = new Map<string, string>()
  const notes: string[] = []
  // Step-by-step progress before the update itself (e.g. "Syncing your GitHub forks 3/14"): only the latest counts.
  let progress: string | undefined
  for (const e of evs) {
    if (e.type === 'resource' && e.id) rows.set(e.id, e.state ?? '')
    else if (e.type === 'snapshot' && e.id) notes.push(t('core.jobProgress.safetyCopySavedId', { id: e.id }))
    else if (e.type === 'health' && e.message) notes.push(e.message)
    else if (e.type === 'status' && e.message) notes.push(e.message)
    else if (e.type === 'progress' && e.message) progress = e.message
  }
  if (progress && rows.size === 0) notes.unshift(progress)
  return (
    <div className="space-y-3">
      <Announcer message={rows.size ? t('core.jobProgress.countSizeResourcesFinished', { count: [...rows.values()].filter((st) => /updated|installed|rolled|^skipped|^failed/.test(st)).length, size: rows.size }) : ''} />
      {[...rows].map(([id, state]) => {
        const done = /updated|installed|rolled/.test(state)
        // Finished, but not as hoped: skipped, failed, waiting for a quiet server, never reached.
        const stopped = /^(skipped|failed|waiting|not attempted)/.test(state)
        return (
          <div key={id} className="flex items-center gap-3 text-sm">
            {done ? <CheckCircle2 className="size-4 text-success" aria-hidden /> : stopped ? <MinusCircle className="size-4 text-amber" aria-hidden /> : <Loader2 className="size-4 animate-spin text-info" aria-hidden />}
            <span className="font-mono text-xs">{id}</span>
            <span className="text-muted-foreground">{state}</span>
          </div>
        )
      })}
      {notes.length > 0 && <ul className="space-y-1 text-sm text-muted-foreground">{notes.map((n, i) => <li key={i}>{n}</li>)}</ul>}
    </div>
  )
}
