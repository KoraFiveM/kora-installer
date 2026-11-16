import { useMemo, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { AddOutcome, AddPlan, ServerDetail } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { ResourceProgress } from '@/components/JobProgress'
import { GROUP_LABEL, groupResources } from '@/lib/catalog'

export function EnsureForm({ server: s, installed }: { server: ServerDetail; installed: string[] }) {
  const [name, setName] = useState('')
  const [restart, setRestart] = useState(false)
  const m = useMutation({ mutationFn: () => api.resources.ensure(s.name, { name: name.trim(), restart }) })
  return (
    <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); if (name.trim()) m.mutate() }}>
      <Field label="Resource name" hint="Adds an ensure line for an installed resource to server.cfg, so it starts with the server.">
        <Input list="installed-resources" value={name} onChange={(e) => setName(e.target.value)} autoComplete="off" />
      </Field>
      <datalist id="installed-resources">{installed.map((r) => <option key={r} value={r} />)}</datalist>
      <Checkbox label="Restart the server now" hint="Otherwise the resource loads the next time the server restarts." checked={restart} onChange={(e) => setRestart(e.target.checked)} />
      <Button type="submit" variant="default" disabled={!name.trim() || m.isPending}>Ensure resource</Button>
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
  const [job, setJob] = useState<string | null>(null)
  const [err, setErr] = useState<unknown>(null)
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
        {plan && plan.items.length > 0 && <Button variant="default" disabled={run.phase === 'running'} onClick={async () => { setErr(null); try { setJob((await api.resources.add(s.name, [...picked])).job) } catch (e) { setErr(e) } }}>Install {plan.items.length} resource{plan.items.length === 1 ? '' : 's'}</Button>}
      </div>
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

export function ResourcesTab({ server: s }: { server: ServerDetail }) {
  const list = useQuery({ queryKey: ['resources', s.name], queryFn: () => api.resources.list(s.name) })
  const installed = list.data ?? []
  return (
    <div className="grid gap-5 lg:grid-cols-2">
      <div className="space-y-5">
        <Card>
          <CardHeader><CardTitle>Installed resources ({installed.length})</CardTitle></CardHeader>
          <CardContent>
            <ErrorNotice error={list.error} />
            <ul className="flex flex-wrap gap-2">{installed.map((r) => <li key={r}><Badge tone="plain"><span className="font-mono">{r}</span></Badge></li>)}</ul>
          </CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>Ensure a resource</CardTitle></CardHeader>
          <CardContent><EnsureForm server={s} installed={installed} /></CardContent>
        </Card>
      </div>
      <Card>
        <CardHeader><CardTitle>Add resources</CardTitle></CardHeader>
        <CardContent><AddResources server={s} /></CardContent>
      </Card>
    </div>
  )
}
