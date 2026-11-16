import { useMemo, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Loader2, Play, RefreshCw, RotateCw, Square } from 'lucide-react'
import { api } from '@/api/client'
import type { AddOutcome, AddPlan, ControlAction, ServerDetail } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useRole, whyNot } from '@/hooks/useRole'
import { useKept } from '@/lib/kept'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { ResourceProgress } from '@/components/JobProgress'
import { GROUP_LABEL, groupResources } from '@/lib/catalog'
import { STATUS_INFO, UpdatesPanel } from './UpdatesPanel'
import { useUpdates, type Updates } from './useUpdates'

export function EnsureForm({ server: s, installed }: { server: ServerDetail; installed: string[] }) {
  const [name, setName] = useState('')
  const [restart, setRestart] = useState(false)
  const m = useMutation({ mutationFn: () => api.resources.ensure(s.name, { name: name.trim(), restart }) })
  const no = whyNot(useRole(), 'operator')
  return (
    <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); if (name.trim()) m.mutate() }}>
      <Field label="Resource name" hint="Adds the resource to server.cfg, so it starts every time the server starts.">
        <Input list="installed-resources" value={name} onChange={(e) => setName(e.target.value)} autoComplete="off" />
      </Field>
      <datalist id="installed-resources">{installed.map((r) => <option key={r} value={r} />)}</datalist>
      <Checkbox label="Restart the server now" hint="Otherwise the resource loads the next time the server restarts." checked={restart} onChange={(e) => setRestart(e.target.checked)} />
      <Button type="submit" variant="default" disabled={!name.trim() || m.isPending || no !== null} title={no ?? undefined}>Add to server.cfg</Button>
      {no && <p className="text-xs text-muted-foreground">{no}</p>}
      {m.isSuccess && <Alert tone="success">{m.data.message}</Alert>}
      <ErrorNotice error={m.error} />
    </form>
  )
}

function PlanView({ plan, dev }: { plan: AddPlan; dev: boolean }) {
  return (
    <div className="space-y-3">
      {plan.already.length > 0 && <p className="text-sm text-muted-foreground">Already installed, so skipped: {plan.already.join(', ')}</p>}
      {plan.items.length === 0 ? <Alert tone="info">Nothing new to install.</Alert> : (
        <ul className="divide-y divide-border rounded-md border border-border">
          {plan.items.map((i) => (
            <li key={i.id} className="flex flex-wrap items-center gap-2 p-3 text-sm">
              <span className="font-mono text-xs">{i.id}</span>
              {i.dependency && <Badge tone="info">Needed by another resource</Badge>}
              {i.fork ? <Badge tone="plain">Your own fork (follows the latest changes)</Badge> : <Badge tone="plain">{i.commit ? `Fixed version ${i.commit.slice(0, 7)}` : `Latest on ${i.branch}`}</Badge>}
              {i.third_party && <Badge tone="orange">Made by someone else</Badge>}
              <span className="ml-auto font-mono text-xs text-muted-foreground">resources/{i.rel}</span>
            </li>
          ))}
        </ul>
      )}
      {plan.ensure.length > 0 && (
        <div className="space-y-1">
          <div className="text-sm font-semibold">Lines added to server.cfg</div>
          <pre className="rounded-md border border-border bg-ink p-3 font-mono text-xs">{plan.ensure.map((e) => `ensure ${e}`).join('\n')}</pre>
        </div>
      )}
      {dev && <p className="text-xs text-muted-foreground">On a development server, Kora's own resources are forked into your GitHub account so you can change them.</p>}
    </div>
  )
}

function AddResources({ server: s }: { server: ServerDetail }) {
  const qc = useQueryClient()
  const cat = useQuery({ queryKey: ['server-catalog', s.name], queryFn: () => api.resources.catalog(s.name) })
  const [query, setQuery] = useState('')
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [plan, setPlan] = useState<AddPlan | null>(null)
  // Kept for the server, so an install that is still running is shown again after opening another tab.
  const [job, setJob] = useKept<string | null>(`add:${s.name}:job`, null)
  const [err, setErr] = useState<unknown>(null)
  const no = whyNot(useRole(), 'admin')
  const run = useJob<AddOutcome>(job, () => {
    void qc.invalidateQueries({ queryKey: ['resources', s.name] })
    void qc.invalidateQueries({ queryKey: ['server-catalog', s.name] })
  })
  const groups = useMemo(() => groupResources(cat.data?.catalog.resources ?? [], query), [cat.data, query])
  const installed = new Set(cat.data?.installed ?? [])
  const toggle = (id: string) => { setPlan(null); setPicked((p) => { const n = new Set(p); if (n.has(id)) n.delete(id); else n.add(id); return n }) }
  return (
    <div className="space-y-4">
      {cat.data?.note && <p className="text-sm text-muted-foreground">{cat.data.note}</p>}
      <ErrorNotice error={cat.error} />
      <Field label="Search the catalogue"><Input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="For example: inventory" /></Field>
      <div className="max-h-96 space-y-4 overflow-y-auto rounded-md border border-border p-3">
        {groups.length === 0 && !cat.isLoading && <p className="text-sm text-muted-foreground">Nothing matches your search.</p>}
        {groups.map(([g, items]) => (
          <fieldset key={g} className="space-y-2">
            <legend className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{GROUP_LABEL[g]}</legend>
            {items.map((r) => (
              <Checkbox key={r.id} label={<>{r.title ?? r.id} <span className="font-mono text-xs text-muted-foreground">{r.id}</span> {installed.has(r.id) && <Badge tone="success">Installed</Badge>}</>} hint={r.description} checked={picked.has(r.id)} disabled={installed.has(r.id)} onChange={() => toggle(r.id)} />
            ))}
          </fieldset>
        ))}
      </div>
      <div className="flex flex-wrap gap-2">
        <Button disabled={picked.size === 0} onClick={async () => { setErr(null); try { setPlan(await api.resources.plan(s.name, [...picked])) } catch (e) { setErr(e) } }}>Preview what will be added</Button>
        {plan && plan.items.length > 0 && <Button variant="default" disabled={run.phase === 'running' || no !== null} title={no ?? undefined} onClick={async () => { setErr(null); try { setJob((await api.resources.add(s.name, [...picked])).job) } catch (e) { setErr(e) } }}>Install {plan.items.length} resource{plan.items.length === 1 ? '' : 's'}</Button>}
      </div>
      {no && <p className="text-xs text-muted-foreground">{no}</p>}
      {plan && <PlanView plan={plan} dev={s.mode === 'development'} />}
      <ErrorNotice error={err ?? (run.phase === 'failed' ? run.error : null)} />
      {run.phase !== 'idle' && <ResourceProgress events={run.events} />}
      {run.phase === 'done' && run.result && (
        <Alert tone={run.result.failed.length ? 'warning' : 'success'} title={run.result.failed.length ? 'Some resources could not be installed' : 'Resources installed'}>
          {run.result.installed.length > 0 && <p>Installed: {run.result.installed.join(', ')}</p>}
          {run.result.failed.map(([id, why]) => <p key={id}>{id}: {why}</p>)}
          {run.result.notes.map((n, i) => <p key={i}>{n}</p>)}
        </Alert>
      )}
    </div>
  )
}

const CONTROL: Record<ControlAction, { label: string; Icon: typeof Play }> = {
  start: { label: 'Start', Icon: Play },
  stop: { label: 'Stop', Icon: Square },
  restart: { label: 'Restart', Icon: RotateCw },
}

/** The resource Kora sends commands through. Stopping it would cut the panel off from the server. */
const CONSOLE_RESOURCE = 'kora_console'

/** One installed resource: its name, what the last check found, and the buttons to check, start, stop and restart it. */
function InstalledResource({ id, updates: u, noCheck, noControl, pending, said, onControl }: {
  id: string
  updates: Updates
  /** Why checking is off (null = allowed). */
  noCheck: string | null
  /** Why start, stop and restart are off (null = allowed). */
  noControl: string | null
  /** The action being sent for this resource right now. */
  pending?: ControlAction
  /** What the last action on this resource answered. */
  said?: { ok: boolean; text: string }
  onControl: (action: ControlAction) => void
}) {
  const row = u.byId.get(id)
  const info = row ? STATUS_INFO[row.status] : null
  const checking = u.checkingRow === id
  return (
    <li className="py-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span className="min-w-0 flex-1 break-all font-mono text-sm">{id}</span>
        {checking ? <span className="text-xs text-muted-foreground" role="status">Checking...</span> : info && <Badge tone={info.tone}>{info.text}</Badge>}
        <div className="flex items-center gap-1">
          <Button size="icon" variant="ghost" className="size-8" disabled={u.busy || noCheck !== null} aria-busy={checking} aria-label={`Check ${id} for updates`} title={noCheck ?? 'Check for updates'} onClick={() => void u.checkOne(id)}>
            {checking ? <Loader2 className="animate-spin" aria-hidden /> : <RefreshCw aria-hidden />}
          </Button>
          {(Object.keys(CONTROL) as ControlAction[]).map((a) => {
            const { label, Icon } = CONTROL[a]
            const why = noControl ?? (a === 'stop' && id === CONSOLE_RESOURCE ? 'Kora uses this resource to send commands to the server, so it cannot be stopped from here.' : null)
            return (
              <Button key={a} size="icon" variant="ghost" className="size-8" disabled={why !== null || pending !== undefined} aria-label={`${label} ${id}`} title={why ?? label} onClick={() => onControl(a)}>
                {pending === a ? <Loader2 className="animate-spin" aria-hidden /> : <Icon aria-hidden />}
              </Button>
            )
          })}
        </div>
      </div>
      {said && <p className={`mt-1 text-xs ${said.ok ? 'text-muted-foreground' : 'text-red'}`} role="status">{said.text}</p>}
    </li>
  )
}

function InstalledList({ server: s, installed, updates: u }: { server: ServerDetail; installed: string[]; updates: Updates }) {
  const role = useRole()
  const noOperator = whyNot(role, 'operator')
  const noControl = noOperator
    ?? (!s.running ? 'The server is not running. Start the server first; its resources start with it.' : null)
    ?? (!s.console_enabled ? 'Turn on the live console in the Console tab first. Kora sends these commands through it.' : null)
  const [filter, setFilter] = useState('')
  const [ask, setAsk] = useState<{ id: string; action: ControlAction } | null>(null)
  const [said, setSaid] = useKept<Record<string, { ok: boolean; text: string }>>(`control:${s.name}:said`, {})
  const control = useMutation({
    mutationFn: (v: { id: string; action: ControlAction }) => api.resources.control(s.name, v.id, v.action),
    onSuccess: (r, v) => setSaid((x) => ({ ...x, [v.id]: { ok: true, text: r.message } })),
    onError: (e, v) => setSaid((x) => ({ ...x, [v.id]: { ok: false, text: (e as { message?: string } | null)?.message ?? `Could not ${v.action} ${v.id}.` } })),
  })
  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase()
    return q ? installed.filter((r) => r.toLowerCase().includes(q)) : installed
  }, [installed, filter])
  return (
    <div className="space-y-3">
      {installed.length > 8 && (
        <Input type="search" aria-label="Filter installed resources" placeholder="Filter by name" value={filter} onChange={(e) => setFilter(e.target.value)} />
      )}
      {noControl && installed.length > 0 && <p className="text-xs text-muted-foreground">Start, stop and restart are off: {noControl}</p>}
      {shown.length === 0 && installed.length > 0 && <p className="text-sm text-muted-foreground">No resource matches "{filter.trim()}".</p>}
      <ul className="divide-y divide-border">
        {shown.map((r) => (
          <InstalledResource
            key={r}
            id={r}
            updates={u}
            noCheck={noOperator}
            noControl={noControl}
            pending={control.isPending && control.variables?.id === r ? control.variables.action : undefined}
            said={said[r]}
            // Starting harms nothing. Stopping or restarting can interrupt players, so it is asked first.
            onControl={(action) => (action === 'start' ? control.mutate({ id: r, action }) : setAsk({ id: r, action }))}
          />
        ))}
      </ul>
      <ConfirmDialog
        open={!!ask}
        onOpenChange={(o) => { if (!o) setAsk(null) }}
        title={ask?.action === 'stop' ? `Stop ${ask.id}?` : `Restart ${ask?.id ?? ''}?`}
        description={ask?.action === 'stop'
          ? 'The resource stops for everyone on the server until it is started again. Other resources that need it may stop working. The server itself keeps running.'
          : 'The resource stops and starts again. Players may notice a short interruption in what it does. The server itself keeps running.'}
        confirmLabel={ask?.action === 'stop' ? 'Stop resource' : 'Restart resource'}
        destructive={ask?.action === 'stop'}
        onConfirm={() => {
          if (ask) control.mutate(ask)
          setAsk(null)
        }}
      />
    </div>
  )
}

export function ResourcesTab({ server: s }: { server: ServerDetail }) {
  const list = useQuery({ queryKey: ['resources', s.name], queryFn: () => api.resources.list(s.name) })
  const installed = list.data ?? []
  const updates = useUpdates(s.name)
  return (
    <div className="space-y-5">
      <UpdatesPanel server={s} updates={updates} />
      <div className="grid gap-5 lg:grid-cols-2">
        <div className="space-y-5">
          <Card>
            <CardHeader><CardTitle>Installed resources ({installed.length})</CardTitle></CardHeader>
            <CardContent>
              <ErrorNotice error={list.error} />
              {list.isLoading && <p className="text-sm text-muted-foreground">Loading...</p>}
              {list.data?.length === 0 && <p className="text-sm text-muted-foreground">No resources are installed yet. Add some from the catalogue.</p>}
              <InstalledList server={s} installed={installed} updates={updates} />
            </CardContent>
          </Card>
          <Card>
            <CardHeader><CardTitle>Start a resource with the server</CardTitle></CardHeader>
            <CardContent><EnsureForm server={s} installed={installed} /></CardContent>
          </Card>
        </div>
        <Card>
          <CardHeader><CardTitle>Add resources</CardTitle></CardHeader>
          <CardContent><AddResources server={s} /></CardContent>
        </Card>
      </div>
    </div>
  )
}
