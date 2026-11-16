import { CheckCircle2, Circle, Loader2, MinusCircle, XCircle } from 'lucide-react'
import type { ProgressEvent, StepStatus } from '@/api/types'
import { Progress } from '@/components/ui/progress'
import { cn } from '@/lib/utils'

const STATUS_TEXT: Record<StepStatus, string> = { pending: 'Waiting', running: 'In progress', done: 'Done', skipped: 'Skipped', failed: 'Failed' }

function Icon({ status }: { status: StepStatus }) {
  const c = 'size-4 shrink-0'
  if (status === 'done') return <CheckCircle2 className={cn(c, 'text-success')} aria-hidden />
  if (status === 'running') return <Loader2 className={cn(c, 'animate-spin text-info')} aria-hidden />
  if (status === 'failed') return <XCircle className={cn(c, 'text-destructive')} aria-hidden />
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
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <Progress value={percent} label="Overall progress" className="flex-1" />
        <span className="w-10 text-right text-sm tabular-nums text-muted-foreground">{percent}%</span>
      </div>
      <ol className="space-y-2">
        {rows.map((s) => (
          <li key={s.step} className="flex items-start gap-3 text-sm">
            <Icon status={s.status} />
            <div className="min-w-0 flex-1">
              <div className={cn(s.status === 'pending' && 'text-muted-foreground')}>
                {s.title} <span className="sr-only">: {STATUS_TEXT[s.status]}</span>
              </div>
              {s.detail && s.status !== 'done' && <div className="text-xs text-muted-foreground">{s.detail}</div>}
            </div>
          </li>
        ))}
      </ol>
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
  for (const e of evs) {
    if (e.type === 'resource' && e.id) rows.set(e.id, e.state ?? '')
    else if (e.type === 'snapshot' && e.id) notes.push(`Safety copy saved (${e.id}). You can roll back to it.`)
    else if (e.type === 'health' && e.message) notes.push(e.message)
    else if (e.type === 'status' && e.message) notes.push(e.message)
  }
  return (
    <div className="space-y-3" aria-live="polite">
      {[...rows].map(([id, state]) => {
        const done = /updated|installed|rolled/.test(state)
        return (
          <div key={id} className="flex items-center gap-3 text-sm">
            {done ? <CheckCircle2 className="size-4 text-success" aria-hidden /> : <Loader2 className="size-4 animate-spin text-info" aria-hidden />}
            <span className="font-mono text-xs">{id}</span>
            <span className="text-muted-foreground">{state}</span>
          </div>
        )
      })}
      {notes.length > 0 && <ul className="space-y-1 text-sm text-muted-foreground">{notes.map((n, i) => <li key={i}>{n}</li>)}</ul>}
    </div>
  )
}
