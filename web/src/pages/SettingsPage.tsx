import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { UninstallItem } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useRole, whyNot } from '@/hooks/useRole'
import { ShortcutsCard } from '@/components/Shortcuts'
import { LicenceKeysCard, RememberedCard } from '@/components/LicenceKeys'
import { RemoteAccessCard, SignedInCard } from '@/components/RemoteAccess'
import { AuditLogCard } from '@/components/AuditLog'
import { JobHistoryCard } from '@/components/JobHistory'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { Input, Select } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { GithubSignIn } from '@/components/GithubSignIn'
import { ResourceProgress } from '@/components/JobProgress'
import { restartPanel } from '@/lib/restart'

function RestartNow({ version }: { version?: string }) {
  const [state, setState] = useState<'idle' | 'restarting' | 'timeout'>('idle')
  const [err, setErr] = useState<unknown>(null)
  const go = async () => {
    setErr(null); setState('restarting')
    try {
      if (await restartPanel()) window.location.reload()
      else setState('timeout')
    } catch (e) { setErr(e); setState('idle') }
  }
  return (
    <div className="space-y-3">
      <Alert tone="success">Updated{version ? ` to ${version}` : ''}.</Alert>
      {state === 'restarting' ? <p className="text-sm">Restarting…</p> : <Button onClick={() => void go()}>Restart now</Button>}
      {state === 'timeout' && <Alert tone="warning">The panel did not come back. Run <code>kora panel</code> on the server.</Alert>}
      <ErrorNotice error={err} />
    </div>
  )
}

function SelfUpdate() {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: ['self-update'], queryFn: api.selfUpdate.check })
  const sys = useQuery({ queryKey: ['system'], queryFn: api.system })
  const [version, setVersion] = useState('')
  const [job, setJob] = useState<string | null>(null)
  const [err, setErr] = useState<unknown>(null)
  const role = useRole()
  const blocked = whyNot(role, 'admin', true)
  const run = useJob(job, () => { void qc.invalidateQueries({ queryKey: ['meta'] }); void qc.invalidateQueries({ queryKey: ['self-update'] }) })
  const go = async (v?: string) => { setErr(null); try { setJob((await api.selfUpdate.apply(v)).job) } catch (e) { setErr(e) } }
  return (
    <Card>
      <CardHeader><CardTitle>Kora version</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap items-center gap-3 text-sm">
          <span>Installed: <strong>{q.data?.current ?? '...'}</strong></span>
          {q.data && (q.data.available ? <Badge tone="info">Version {q.data.latest} is available</Badge> : <Badge tone="plain">Up to date</Badge>)}
          <Button size="sm" onClick={() => void q.refetch()} disabled={q.isFetching}>Check again</Button>
        </div>
        <div className="flex flex-wrap items-end gap-3">
          <Button variant={q.data?.available ? 'default' : 'secondary'} disabled={run.phase === 'running' || !q.data?.latest || blocked !== null} onClick={() => go()}>Update to the latest</Button>
          <div className="w-48"><Field label="Or a specific version"><Select value={version} onChange={(e) => setVersion(e.target.value)}><option value="">Choose...</option>{(q.data?.versions ?? []).map((v) => <option key={v}>{v}</option>)}</Select></Field></div>
          <Button disabled={!version || run.phase === 'running' || blocked !== null} onClick={() => go(version)}>Install this version</Button>
        </div>
        {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
        {run.phase !== 'idle' && <ResourceProgress events={run.events} />}
        {run.phase === 'done' && (sys.data?.can_restart
          ? <RestartNow version={q.data?.current} />
          : <Alert tone="success">Updated. This panel runs in a terminal window: stop it (Ctrl+C) and run <code>kora panel</code> again to use the new version.</Alert>)}
        {run.phase === 'failed' && <ErrorNotice error={run.error} />}
        <ErrorNotice error={err ?? q.error} />
      </CardContent>
    </Card>
  )
}

function Uninstall() {
  const role = useRole()
  const [items, setItems] = useState<UninstallItem[] | null>(null)
  const [typed, setTyped] = useState('')
  const [job, setJob] = useState<string | null>(null)
  const plan = useMutation({ mutationFn: api.uninstall.plan, onSuccess: (p) => setItems(p.items) })
  const run = useJob<{ notes?: string[] }>(job)
  const start = useMutation({
    mutationFn: () => api.uninstall.run({
      answers: (items ?? []).map((i) => ({ item: i.item, remove: i.remove })),
      delete_files: !!items?.some((i) => i.kind === 'folder' && i.remove),
      delete_databases: !!items?.some((i) => i.kind === 'database' && i.remove),
      only: [],
    }),
    onSuccess: (r) => setJob(r.job),
  })
  const danger = (items ?? []).some((i) => i.remove && (i.kind === 'folder' || i.kind === 'database'))
  if (!role.can('admin')) return null
  return (
    <Card className="border-destructive/50">
      <CardHeader><CardTitle>Uninstall Kora</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">Removes what Kora added to this computer. By default your server folders and databases are kept.</p>
        {!role.canDestroy && <p className="text-xs text-muted-foreground">{whyNot(role, 'admin', true)}</p>}
        {!items && <Button onClick={() => plan.mutate()} disabled={plan.isPending || !role.canDestroy}>Show what would be removed</Button>}
        {items && !job && (
          <div className="space-y-4">
            <fieldset className="space-y-3">
              <legend className="mb-1 text-sm font-semibold">Remove these</legend>
              {items.map((i, idx) => (
                <Checkbox key={i.item} label={i.description} hint={i.kind === 'folder' || i.kind === 'database' ? 'This deletes data and cannot be undone.' : undefined} checked={i.remove} onChange={(e) => setItems(items.map((x, k) => (k === idx ? { ...x, remove: e.target.checked } : x)))} />
              ))}
            </fieldset>
            {danger && <Alert tone="danger">You chose to delete server folders or databases. Everything in them will be lost.</Alert>}
            <Field label='Type "uninstall" to confirm'><Input value={typed} onChange={(e) => setTyped(e.target.value)} autoComplete="off" /></Field>
            <Button variant="destructive" disabled={typed !== 'uninstall' || start.isPending} onClick={() => start.mutate()}>Uninstall</Button>
          </div>
        )}
        {job && <ResourceProgress events={run.events} />}
        {run.phase === 'done' && <Alert tone="success" title="Uninstall finished">{run.result?.notes?.join(' ')}</Alert>}
        {run.phase === 'failed' && <ErrorNotice error={run.error} />}
        <ErrorNotice error={plan.error ?? start.error} />
      </CardContent>
    </Card>
  )
}

export default function SettingsPage() {
  const meta = useQuery({ queryKey: ['meta'], queryFn: api.meta })
  const sys = useQuery({ queryKey: ['system'], queryFn: api.system })
  const role = useRole()
  return (
    <div className="mx-auto max-w-3xl space-y-5">
      <h1 className="font-heading text-2xl font-bold">Settings</h1>
      {role.remote ? <SignedInCard /> : <RemoteAccessCard />}
      <SelfUpdate />
      <ShortcutsCard />
      <LicenceKeysCard />
      <RememberedCard />
      <JobHistoryCard />
      <AuditLogCard />
      <Card><CardHeader><CardTitle>GitHub account</CardTitle></CardHeader><CardContent><GithubSignIn /></CardContent></Card>
      <Uninstall />
      <Card>
        <CardHeader><CardTitle>About</CardTitle></CardHeader>
        <CardContent className="space-y-2 text-sm">
          <p>Kora Panel {meta.data?.version}. It manages FiveM servers on {meta.data?.platform === 'windows' ? 'Windows' : 'Linux'}{sys.data?.helper_installed ? ', with the administrator helper installed' : ''}.</p>
          <p className="text-muted-foreground">Website: koraframe.work</p>
          <p className="text-muted-foreground">Documentation: docs.koraframe.work/guides/installer/overview</p>
          <p className="text-muted-foreground">Source and releases: <a className="text-info underline" href="https://github.com/KoraFiveM/kora-installer" target="_blank" rel="noopener noreferrer">github.com/KoraFiveM/kora-installer</a></p>
          <p className="text-muted-foreground">This page makes no requests to other websites. Everything it shows comes from Kora on this computer.</p>
        </CardContent>
      </Card>
    </div>
  )
}
