import { useCallback, useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { ExternalLink, Loader2 } from 'lucide-react'
import { api } from '@/api/client'
import type { InstallOutcome, InstallRequest, ProgressEvent } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useServerControl } from '@/hooks/useServerControl'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { CopyButton, CopyField } from '@/components/CopyButton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { StepProgress } from '@/components/JobProgress'
import { Skeleton } from '@/components/Skeleton'
import { formatBytes } from '@/lib/utils'
import { REMOTE_ACTIONS_COMMAND, useInstallBlock } from './install/access'
import { applySaved, buildRequest, initialForm, STEP_LABEL, stepProblem, stepsFor, type StepId, type WizardForm } from './install/form'
import { BuildStep, CfxStep, DatabaseStep, GithubStep, LicenceStep, ModeStep, NameStep, OptionsStep, PortsStep, ResourcesStep } from './install/Steps'
import { cn } from '@/lib/utils'

function ReviewStep({ f, req, onDisk }: { f: WizardForm; req: InstallRequest; onDisk: (ok: boolean) => void }) {
  const safeKey = JSON.stringify([req.dir, req.mode, req.firewall, req.service, req.db, req.txadmin_public])
  const disk = useQuery({ queryKey: ['disk', req.dir], queryFn: () => api.installs.checkDisk(req.dir) })
  useEffect(() => onDisk(disk.data ? disk.data.ok : true), [disk.data, onDisk])
  const plan = useQuery({ queryKey: ['plan', safeKey], queryFn: () => api.installs.plan({ ...req, dry_run: true }) })
  const rows: [string, string][] = [
    ['Mode', f.mode === 'production' ? 'Production' : 'Development'],
    ['Name', req.name],
    ['Folder', req.dir],
    ['Server name for players', req.hostname || '(default)'],
    ['Ports', `Game ${req.ports.game}, txAdmin ${req.ports.txadmin}`],
    ['FiveM server build', f.artifact === 'recommended' ? 'Recommended' : f.artifact === 'pick' ? `Build ${f.artifactBuild}` : 'Custom download'],
    ['Resources', f.resourcesAll ? 'Everything' : `${f.resourceIds.length} picked, plus what they need`],
    ['Database', f.db === 'use_existing' ? 'Use the existing database' : f.db === 'install_if_missing' ? 'Set up for me' : 'Skip'],
    ['Database login', f.db === 'skip' ? 'Not needed' : f.dbCustom ? `Typed (user ${f.dbUser}, password hidden)` : 'Automatic'],
    ['Firewall', req.firewall ? (req.txadmin_public ? 'Open the game and txAdmin ports' : 'Open the game port') : 'Leave closed'],
    ['Background service', req.service ? 'Yes' : 'No'],
    ['Licence key', 'Provided (hidden)'],
    ['txAdmin admin', f.cfxFound ? (f.cfxFound.username ? `${f.cfxFound.username} (${f.cfxFound.identifier})` : f.cfxFound.identifier) : 'A generated local account'],
    ...(f.mode === 'development' ? ([['GitHub forks go to', f.githubOwner]] as [string, string][]) : []),
  ]
  return (
    <div className="space-y-4">
      <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
        {rows.map(([k, v]) => (<div key={k} className="contents"><dt className="text-muted-foreground">{k}</dt><dd className="break-all">{v}</dd></div>))}
      </dl>
      {disk.data && <Alert tone={disk.data.ok ? 'success' : 'danger'} title="Disk space">{disk.data.message}{!disk.data.message.includes('GB') && ` ${formatBytes(disk.data.free_bytes)} free.`}</Alert>}
      <ErrorNotice error={disk.error ?? plan.error} />
      {plan.data && (
        <Alert tone={plan.data.needs_admin.length ? 'warning' : 'info'} title="Administrator rights">
          {plan.data.needs_admin.length === 0 ? 'Nothing in this install needs administrator rights.' : (
            <>
              <p>These steps need administrator rights. Kora asks for them when needed, or tells you the exact command to run:</p>
              <ul className="mt-1 list-disc pl-5">{plan.data.needs_admin.map((n) => <li key={n}>{n}</li>)}</ul>
            </>
          )}
        </Alert>
      )}
      <p className="text-xs text-muted-foreground">{plan.data ? `${plan.data.steps.length} steps will run.` : ''} Your licence key and database password are sent to Kora on this computer only and are never shown again.</p>
    </div>
  )
}

export function Outcome({ o, ports, cfx }: { o: InstallOutcome; ports: { txadmin: number }; cfx?: string | null }) {
  const ctl = useServerControl(o.name)
  const tx = `http://${window.location.hostname}:${ports.txadmin}`
  return (
    <div className="space-y-4">
      <Alert tone={o.running ? 'success' : 'warning'} title={o.running ? 'Your server is installed and running' : 'Your server is installed'}>
        {o.running ? 'Players can connect now.' : o.launch_problem ?? 'It was not started. Start it from the server page.'}
      </Alert>
      {!o.running && o.log_tail.length > 0 && <pre className="max-h-48 overflow-auto rounded-md border border-border bg-ink p-3 font-mono text-xs">{o.log_tail.join('\n')}</pre>}
      <div className="space-y-1.5"><div className="text-sm text-muted-foreground">Connect address</div><CopyField value={o.connect} label="Connect address" /></div>
      <div className="space-y-1.5"><div className="text-sm text-muted-foreground">txAdmin</div><CopyField value={tx} label="txAdmin address" /></div>
      {!o.admin_user && cfx && (
        <p className="text-sm">Sign in to txAdmin with your Cfx.re account (<strong>{cfx}</strong>). No password was generated.</p>
      )}
      {o.admin_user && (
        <p className="text-sm">txAdmin administrator: <strong>{o.admin_user}</strong>. Its password was saved in <code className="font-mono text-xs">{o.admin_password_file}</code>. Open that file on this computer to read it, and change the password after you sign in.</p>
      )}
      {o.notes.length > 0 && <ul className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">{o.notes.map((n, i) => <li key={i}>{n}</li>)}</ul>}
      <div className="flex flex-wrap gap-2">
        <Button asChild variant="default"><a href={tx} target="_blank" rel="noopener noreferrer"><ExternalLink /> Open txAdmin</a></Button>
        <CopyButton text={o.connect} label="Copy connect address" size="default" />
        <Button disabled={ctl.busy} onClick={() => ctl.run('stop')}>Stop server</Button>
        <Button asChild><Link to={`/servers/${encodeURIComponent(o.name)}`}>Go to the server</Link></Button>
      </div>
    </div>
  )
}

/**
 * Installing is admin-only and, for remote sign-ins, off unless switched on. Say so before step 1
 * instead of after the last one. The steps (and their admin-only page-load requests) only mount
 * once the sign-in is known and allowed.
 */
export default function InstallWizard() {
  // Same answer the app shell keeps fresh; asked here too so a direct visit waits for it.
  const meta = useQuery({ queryKey: ['meta'], queryFn: api.meta, staleTime: 15000 })
  const block = useInstallBlock()
  if (meta.isPending) return <Skeleton className="h-64" />
  if (block) {
    return (
      <div className="mx-auto max-w-2xl space-y-5">
        <h1 className="font-heading text-2xl font-bold">Install a new server</h1>
        <Alert tone="warning" title="You cannot install a server from here">
          {block.kind === 'remote' ? (
            <>
              <p>Installing a server is turned off for remote sign-ins. To turn it on, run this on the computer that runs Kora (over SSH is fine):</p>
              <p className="my-2"><code className="rounded bg-ink px-2 py-1 font-mono text-xs">{REMOTE_ACTIONS_COMMAND}</code></p>
              <p>Then reload this page.</p>
            </>
          ) : block.reason}
        </Alert>
        <Button asChild><Link to="/">Back to your servers</Link></Button>
      </div>
    )
  }
  return <Wizard />
}

function Wizard() {
  const qc = useQueryClient()
  const defaults = useQuery({ queryKey: ['install-defaults'], queryFn: api.installs.defaults, staleTime: Infinity })
  const appQ = useQuery({ queryKey: ['app'], queryFn: api.app.status })
  const gh = useQuery({ queryKey: ['github'], queryFn: api.github.status })
  const [f, setF] = useState<WizardForm | null>(null)
  const [step, setStep] = useState(0)
  const [advanced, setAdvanced] = useState(false)
  const [portsOk, setPortsOk] = useState(false)
  const [job, setJob] = useState<string | null>(null)
  const [startErr, setStartErr] = useState<unknown>(null)
  const [resumeOffered, setResumeOffered] = useState(true)
  const [starting, setStarting] = useState(false)
  const [diskOk, setDiskOk] = useState(true)
  const arts = useQuery({ queryKey: ['artifacts'], queryFn: api.artifacts })

  useEffect(() => {
    if (defaults.data && !f) setF(initialForm(defaults.data))
  }, [defaults.data, f])

  const run = useJob<InstallOutcome, ProgressEvent>(job, () => {
    void qc.invalidateQueries({ queryKey: ['servers'] })
    void qc.invalidateQueries({ queryKey: ['install-defaults'] })
  })
  const setPortsOkStable = useCallback((ok: boolean) => setPortsOk(ok), [])

  if (defaults.isError) return <ErrorNotice error={defaults.error} />
  if (!f || !defaults.data) return <Skeleton className="h-64" />
  const set = (p: Partial<WizardForm>) => setF((x) => (x ? { ...x, ...p } : x))
  const appSupported = !!appQ.data?.supported
  const artUrl = arts.data?.items.find((a) => a.build === f.artifactBuild)?.url ?? null
  const req = buildRequest(f, { appSupported, artifactUrl: artUrl })
  const steps = stepsFor(f.mode)
  const id: StepId = steps[Math.min(step, steps.length - 1)]
  const problem = stepProblem(id, f, { portsOk, githubSignedIn: !!gh.data?.signed_in })
  const saved = defaults.data.saved
  const common = { f, set, advanced, defaultsDir: defaults.data.dir }

  const start = async () => {
    setStartErr(null)
    setStarting(true)
    try {
      setJob((await api.installs.start(req)).job)
    } catch (e) {
      setStartErr(e)
    } finally {
      setStarting(false)
    }
  }

  // ---- running / finished ----
  if (job) {
    return (
      <div className="mx-auto max-w-2xl space-y-5">
        <h1 className="font-heading text-2xl font-bold">{run.phase === 'done' ? 'All done' : run.phase === 'failed' ? 'The install stopped' : 'Installing your server'}</h1>
        <Card>
          <CardContent className="space-y-5 p-6">
            {run.phase === 'running' && <p className="text-sm text-muted-foreground">This takes a few minutes. You can leave this page open; it keeps going.</p>}
            {(run.phase === 'idle' || run.phase === 'running') && run.events.length === 0 ? (
              <p role="status" className="flex items-center gap-2 text-sm">
                <Loader2 className="size-4 animate-spin text-info" aria-hidden /> Getting ready. The first step shows up in a moment...
              </p>
            ) : (
              <StepProgress events={run.events as ProgressEvent[]} />
            )}
            {run.phase === 'done' && run.result && <Outcome o={run.result} ports={req.ports} cfx={req.cfx_identifier} />}
            {run.phase === 'failed' && (
              <div className="space-y-4">
                <ErrorNotice error={run.error} title="The install could not finish" />
                {run.error?.kept && run.error.kept.length > 0 && (
                  <div className="text-sm"><div className="font-semibold">What was kept</div><ul className="list-disc pl-5 text-muted-foreground">{run.error.kept.map((k) => <li key={k}>{k}</li>)}</ul></div>
                )}
                {run.error?.log_path && <p className="text-sm">Details are in <code className="font-mono text-xs">{run.error.log_path}</code></p>}
                <div className="flex gap-2">
                  <Button variant="default" onClick={() => { setJob(null); void start() }}>Try again</Button>
                  <Button onClick={() => { setJob(null); setStep(steps.length - 1) }}>Change my answers</Button>
                </div>
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    )
  }

  const last = step === steps.length - 1
  return (
    <div className="mx-auto max-w-2xl space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="font-heading text-2xl font-bold">Install a new server</h1>
        <Checkbox label="Show advanced options" checked={advanced} onChange={(e) => setAdvanced(e.target.checked)} />
      </div>
      {saved && resumeOffered && step === 0 && (
        <Alert tone="info" title="Continue where you left off?">
          <p>An earlier install of &quot;{saved.name}&quot; stopped part-way. Kora kept your answers (not your licence key).</p>
          <div className="mt-2 flex gap-2">
            <Button size="sm" onClick={() => { setF(applySaved(f, saved)); setResumeOffered(false); setStep(stepsFor(saved.mode ?? f.mode).indexOf('licence')) }}>Continue where I left off</Button>
            <Button size="sm" onClick={async () => { await api.installs.clearSaved(); setResumeOffered(false); void qc.invalidateQueries({ queryKey: ['install-defaults'] }) }}>Start fresh</Button>
          </div>
        </Alert>
      )}
      <ol className="flex flex-wrap gap-x-4 gap-y-1 text-xs" aria-label="Steps">
        {steps.map((s, i) => (
          <li key={s} aria-current={i === step ? 'step' : undefined} className={cn(i === step ? 'font-semibold text-foreground' : 'text-muted-foreground')}>{i + 1}. {STEP_LABEL[s]}</li>
        ))}
      </ol>
      <Card>
        <CardHeader><CardTitle className="text-lg">{STEP_LABEL[id]}</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          {id === 'mode' && <ModeStep {...common} />}
          {id === 'name' && <NameStep {...common} />}
          {id === 'ports' && <PortsStep {...common} onOk={setPortsOkStable} />}
          {id === 'build' && <BuildStep {...common} />}
          {id === 'resources' && <ResourcesStep {...common} />}
          {id === 'database' && <DatabaseStep {...common} />}
          {id === 'options' && <OptionsStep {...common} appSupported={appSupported} />}
          {id === 'licence' && <LicenceStep {...common} />}
          {id === 'cfx' && <CfxStep {...common} />}
          {id === 'github' && <GithubStep {...common} signedIn={!!gh.data?.signed_in} onSignedIn={() => void gh.refetch()} />}
          {id === 'review' && <ReviewStep f={f} req={req} onDisk={setDiskOk} />}
          <ErrorNotice error={startErr} />
          {starting && (
            <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" aria-hidden /> Checking your answers (ports, GitHub, permissions) and starting the install. This can take a few seconds.
            </p>
          )}
          {problem && !last && <p className="text-sm text-muted-foreground">{problem}</p>}
          <div className="flex justify-between pt-2">
            <Button disabled={step === 0} onClick={() => setStep((s) => s - 1)}>Back</Button>
            {last ? (
              <Button variant="default" disabled={starting || !diskOk} aria-busy={starting} onClick={start}>
                {starting ? <><Loader2 className="animate-spin" aria-hidden /> Starting...</> : 'Install the server'}
              </Button>
            ) : (
              <Button variant="default" disabled={!!problem} onClick={() => setStep((s) => s + 1)}>Next</Button>
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
