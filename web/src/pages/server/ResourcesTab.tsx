import { useMemo, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Loader2 } from 'lucide-react'
import { api } from '@/api/client'
import type { AddOutcome, AddPlan, ResourceChange, ServerDetail, UpdatePreview, UpdateReport } from '@/api/types'
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
import { ReportView, reportHasProblems, shortSha, STATUS_INFO, UpdatesPanel } from './UpdatesPanel'

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

/** One installed resource. Its name checks just this resource; the result shows under the row, with the same apply as the update list when it is a simple update. */
function InstalledResource({ server: s, id }: { server: ServerDetail; id: string }) {
  const qc = useQueryClient()
  const [checkJob, setCheckJob] = useState<string | null>(null)
  const [applyJob, setApplyJob] = useState<string | null>(null)
  const [result, setResult] = useState<ResourceChange | null>(null)
  const [checked, setChecked] = useState(false)
  const [err, setErr] = useState<unknown>(null)
  const check = useJob<UpdatePreview>(checkJob, (j) => {
    if (j.state === 'done' && j.result) {
      setResult(j.result.resources.find((r) => r.id === id) ?? null)
      setChecked(true)
    }
  })
  const apply = useJob<UpdateReport>(applyJob, () => {
    void qc.invalidateQueries({ queryKey: ['snapshots', s.name] })
    void qc.invalidateQueries({ queryKey: ['server', s.name] })
  })
  const busy = check.phase === 'running' || apply.phase === 'running'
  const info = result ? STATUS_INFO[result.status] : null

  const doCheck = async () => {
    setErr(null)
    setApplyJob(null)
    setChecked(false)
    try { setCheckJob((await api.update.preview(s.name, { resources: [id] })).job) } catch (e) { setErr(e) }
  }
  const doApply = async () => {
    if (!result || result.status !== 'clean') return
    setErr(null)
    setResult(null)
    try {
      // The version shown here, and nothing else. Restarts only when nobody is playing, as the default in the update list.
      setApplyJob((await api.update.apply(s.name, { scope: { resources: [id] }, restart: 'when_quiet', skip: [], targets: { [id]: result.to } })).job)
    } catch (e) { setErr(e) }
  }

  return (
    <li className="space-y-2 py-3">
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" variant="ghost" className="font-mono" disabled={busy} onClick={() => void doCheck()} aria-busy={check.phase === 'running'} title="Check only this resource for updates">
          {check.phase === 'running' && <Loader2 className="animate-spin" aria-hidden />}
          {id}
        </Button>
      </div>
      <ErrorNotice error={err ?? (check.phase === 'failed' ? check.error : null)} />
      {checked && result?.status === 'up_to_date' && <p className="text-sm text-muted-foreground">Up to date.</p>}
      {checked && !result && check.phase === 'done' && <p className="text-sm text-muted-foreground">Kora did not find this resource to check.</p>}
      {result && result.status !== 'up_to_date' && info && (
        <div className="space-y-2 rounded-md border border-border p-3 text-sm">
          <Badge tone={info.tone}>{info.text}</Badge>
          <div className="text-muted-foreground"><code className="font-mono text-xs">{shortSha(result.from)}</code> to <code className="font-mono text-xs">{shortSha(result.to)}</code>{result.behind ? `, ${result.behind} new change${result.behind === 1 ? '' : 's'}` : ''}</div>
          {info.why && <p className="text-xs text-orange">{info.why}</p>}
          {result.status === 'clean' && (
            <>
              <p className="text-xs text-muted-foreground">Restarts the server only when nobody is playing.</p>
              <Button size="sm" variant="default" disabled={busy} onClick={() => void doApply()}>Update this resource</Button>
            </>
          )}
        </div>
      )}
      {apply.phase !== 'idle' && <ResourceProgress events={apply.events} />}
      {apply.phase === 'failed' && <ErrorNotice error={apply.error} title="The update did not finish" />}
      {apply.phase === 'done' && apply.result && (
        <>
          {reportHasProblems(apply.result)
            ? <Alert tone="warning" title="Update finished, but some things need a look">See the details below.</Alert>
            : <Alert tone="success" title="Update finished" />}
          <ReportView r={apply.result} />
        </>
      )}
    </li>
  )
}

export function ResourcesTab({ server: s }: { server: ServerDetail }) {
  const list = useQuery({ queryKey: ['resources', s.name], queryFn: () => api.resources.list(s.name) })
  const installed = list.data ?? []
  return (
    <div className="space-y-5">
      <UpdatesPanel server={s} />
      <div className="grid gap-5 lg:grid-cols-2">
        <div className="space-y-5">
          <Card>
            <CardHeader><CardTitle>Installed resources ({installed.length})</CardTitle></CardHeader>
            <CardContent>
              <p className="mb-2 text-xs text-muted-foreground">Click a name to check that resource for updates.</p>
              <ErrorNotice error={list.error} />
              <ul className="divide-y divide-border">{installed.map((r) => <InstalledResource key={r} server={s} id={r} />)}</ul>
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
    </div>
  )
}
