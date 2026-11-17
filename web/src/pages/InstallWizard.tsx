import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { ExternalLink, Loader2 } from 'lucide-react'
import { api } from '@/api/client'
import type { InstallDefaults, InstallOutcome, InstallRequest, ProgressEvent } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useServerControl } from '@/hooks/useServerControl'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { CopyButton, CopyField } from '@/components/CopyButton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { GithubSignIn } from '@/components/GithubSignIn'
import type { ActionHandlers } from '@/lib/problemActions'
import { StepProgress } from '@/components/JobProgress'
import { Skeleton } from '@/components/Skeleton'
import { formatBytes, validatePorts } from '@/lib/utils'
import { REMOTE_ACTIONS_COMMAND, useInstallBlock } from './install/access'
import { allProblems, applySaved, buildRequest, initialForm, quickStepsFor, sectionStep, STEP_LABEL, stepProblem, stepsFor, withoutAdmin, type StepId, type WizardForm } from './install/form'
import { usePortsCheck } from './install/ports'
import { BuildStep, CfxStep, DatabaseStep, GithubStep, LicenceStep, ModeStep, NameStep, OptionsStep, PortsStep, ResourcesStep } from './install/Steps'
import { cn } from '@/lib/utils'

interface ReviewRow {
  label: string
  value: string
  /** The step that "Change" opens. */
  step: StepId
  /** The answer only shows in that step with the advanced options on. */
  advanced?: boolean
}

function reviewRows(f: WizardForm, req: InstallRequest): ReviewRow[] {
  const build = f.artifact === 'recommended' ? 'Recommended' : f.artifact === 'pick' ? (f.artifactBuild === null ? 'Not picked yet' : `Build ${f.artifactBuild}`) : 'Custom download'
  const rows: ReviewRow[] = [
    { label: 'Licence key', value: f.license.trim() ? 'Provided (hidden)' : 'Not given yet', step: 'licence' },
    { label: 'Kind of server', value: f.mode === 'production' ? 'Production (a server for players)' : 'Development (for building and changing resources)', step: 'mode' },
    { label: 'Name', value: req.name || 'Not given yet', step: 'name' },
    { label: 'Folder', value: req.dir || 'Not given yet', step: 'name' },
    { label: 'Server name for players', value: req.hostname || '(default)', step: 'name', advanced: true },
    { label: 'Ports', value: `Game ${f.game.trim() || '?'}, txAdmin ${f.tx.trim() || '?'}`, step: 'ports' },
    { label: 'FiveM server build', value: f.gameBuild.trim() ? `${build}, game build ${f.gameBuild.trim()}` : build, step: 'build' },
    { label: 'Resources', value: f.resourcesAll ? 'Everything' : `${f.resourceIds.length} picked, plus what they need`, step: 'resources' },
    { label: 'Database', value: f.db === 'use_existing' ? 'Use the existing database' : f.db === 'install_if_missing' ? 'Set up for me' : 'Skip', step: 'database' },
    { label: 'Database login', value: f.db === 'skip' ? 'Not needed' : f.dbCustom ? `Typed (user ${f.dbUser}, password hidden)` : 'Automatic', step: 'database' },
    { label: 'Firewall', value: req.firewall ? (req.txadmin_public ? 'Open the game and txAdmin ports' : 'Open the game port') : 'Leave closed', step: 'options' },
    { label: 'Background service', value: req.service ? 'Yes' : 'No', step: 'options' },
  ]
  rows.push({
    label: 'Sign in to txAdmin with your Cfx.re account',
    value: f.cfxFound ? (f.cfxFound.username ? `${f.cfxFound.username} (${f.cfxFound.identifier})` : f.cfxFound.identifier) : 'Not set. Kora makes a local admin with a generated password.',
    step: 'cfx',
  })
  if (f.mode === 'development') rows.push({ label: 'GitHub forks go to', value: f.githubOwner || 'Not chosen yet', step: 'github' })
  return rows
}

/** Windows: said before anything starts, so the prompt is expected (and how to avoid it). */
export function AdminPromptNotice({ needs, onWithout }: { needs: string[]; onWithout?: () => void }) {
  return (
    <Alert tone="info" title="Windows will ask for permission once">
      <p>These parts need administrator rights:</p>
      <ul className="mt-1 list-disc pl-5">{needs.map((n) => <li key={n}>{n}</li>)}</ul>
      <p className="mt-2">When you press Install, Windows asks once for permission on the computer that runs Kora: choose Yes, then watch the install carry on here. If you do not see the prompt, look for a flashing shield icon in the taskbar.</p>
      {onWithout && (
        <p className="mt-2">Rather not? <Button variant="link" className="h-auto p-0" onClick={onWithout}>Install without these parts</Button> (no firewall change, no background service, and an existing database only).</p>
      )}
    </Alert>
  )
}

function ReviewStep({ rows, req, problems, waiting, onChange, onDisk, onWithoutAdmin }: {
  rows: ReviewRow[]
  req: InstallRequest
  /** What still has to be fixed, by step. Shown beside the first line of that step. */
  problems: Partial<Record<StepId, string>>
  /** Steps whose "problem" is only a check that has not answered yet. */
  waiting: StepId[]
  onChange: (row: ReviewRow) => void
  onDisk: (ok: boolean) => void
  /** Drop the parts that need administrator rights (Windows notice). */
  onWithoutAdmin?: () => void
}) {
  const safeKey = JSON.stringify([req.dir, req.mode, req.firewall, req.service, req.db, req.txadmin_public])
  const disk = useQuery({ queryKey: ['disk', req.dir], queryFn: () => api.installs.checkDisk(req.dir) })
  useEffect(() => onDisk(disk.data ? disk.data.ok : true), [disk.data, onDisk])
  const plan = useQuery({ queryKey: ['plan', safeKey], queryFn: () => api.installs.plan({ ...req, dry_run: true }) })
  const said = new Set<StepId>()
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">This is what Kora will do. The suggested answers work for most people. Use Change on any line to pick something else.</p>
      <dl className="divide-y divide-border text-sm">
        {rows.map((r) => {
          const problem = said.has(r.step) ? undefined : problems[r.step]
          said.add(r.step)
          return (
            <div key={r.label} className="grid grid-cols-[1fr_auto] items-start gap-x-4 gap-y-1 py-2 sm:grid-cols-[11rem_1fr_auto]">
              <dt className="col-span-2 text-muted-foreground sm:col-span-1">{r.label}</dt>
              <dd className="min-w-0 space-y-1">
                <div className="break-words">{r.value}</div>
                {problem && (waiting.includes(r.step)
                  ? <p role="status" className="text-muted-foreground">{problem}</p>
                  : <p role="alert" className="font-medium text-destructive">{problem}</p>)}
              </dd>
              <dd><Button variant="link" size="sm" className="h-auto p-0 text-sm" aria-label={`Change ${r.label}`} onClick={() => onChange(r)}>Change</Button></dd>
            </div>
          )
        })}
      </dl>
      {disk.data && <Alert tone={disk.data.ok ? 'success' : 'danger'} title="Disk space">{disk.data.message}{!disk.data.message.includes('GB') && ` ${formatBytes(disk.data.free_bytes)} free.`}</Alert>}
      <ErrorNotice error={disk.error ?? plan.error} />
      {plan.data?.admin_prompt && <AdminPromptNotice needs={plan.data.needs_admin} onWithout={onWithoutAdmin} />}
      {plan.data && !plan.data.admin_prompt && (
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
  const gh = useQuery({ queryKey: ['github-status'], queryFn: api.github.status })
  const [f, setF] = useState<WizardForm | null>(null)
  if (defaults.data && !f) setF(initialForm(defaults.data))
  if (defaults.isError) return <ErrorNotice error={defaults.error} />
  if (!f || !defaults.data) return <Skeleton className="h-64" />
  const set = (p: Partial<WizardForm>) => setF((x) => (x ? { ...x, ...p } : x))
  return <WizardSteps f={f} setF={setF} set={set} defaults={defaults.data} githubSignedIn={!!gh.data?.signed_in} onGithubSignedIn={() => void gh.refetch()} onServersChanged={() => {
    void qc.invalidateQueries({ queryKey: ['servers'] })
    void qc.invalidateQueries({ queryKey: ['install-defaults'] })
  }} />
}

function WizardSteps({ f, setF, set, defaults, githubSignedIn, onGithubSignedIn, onServersChanged }: {
  f: WizardForm
  setF: (f: WizardForm) => void
  set: (p: Partial<WizardForm>) => void
  defaults: InstallDefaults
  githubSignedIn: boolean
  onGithubSignedIn: () => void
  onServersChanged: () => void
}) {
  const qc = useQueryClient()
  const [step, setStep] = useState(0)
  /** false = the short path (licence key, name, check and install). true = every step, one by one. */
  const [everyStep, setEveryStep] = useState(false)
  /** A step opened with "Change" on the last screen. Closing it goes back to that screen. */
  const [editing, setEditing] = useState<StepId | null>(null)
  const [advanced, setAdvanced] = useState(false)
  const [job, setJob] = useState<string | null>(null)
  const [startErr, setStartErr] = useState<unknown>(null)
  const [resumeOffered, setResumeOffered] = useState(true)
  const [starting, setStarting] = useState(false)
  const [diskOk, setDiskOk] = useState(true)
  /** Carrying on with a stopped install did not work (for example: the panel was restarted). */
  const [resumeErr, setResumeErr] = useState<unknown>(null)
  /** A failure offered "Sign in to GitHub": the sign-in is shown under it. */
  const [githubAsked, setGithubAsked] = useState(false)
  const arts = useQuery({ queryKey: ['artifacts'], queryFn: api.artifacts })
  const ports = usePortsCheck(f.game, f.tx)
  const run = useJob<InstallOutcome, ProgressEvent>(job, onServersChanged)

  const artUrl = arts.data?.items.find((a) => a.build === f.artifactBuild)?.url ?? null
  const req = buildRequest(f, { artifactUrl: artUrl })
  const stepsOf = (mode: WizardForm['mode']) => (everyStep ? stepsFor(mode) : quickStepsFor(mode))
  const steps = stepsOf(f.mode)
  const at = Math.min(step, steps.length - 1)
  const id: StepId = steps[at]
  const ctx = { ports: ports.state, githubSignedIn }
  const problems = allProblems(f, ctx)
  const saved = defaults.saved
  const common = { f, set, advanced, defaultsDir: defaults.dir }

  const start = async (opts: { askAdmin?: boolean } = {}) => {
    setStartErr(null)
    setStarting(true)
    try {
      setJob((await api.installs.start(req, opts)).job)
    } catch (e) {
      setStartErr(e)
    } finally {
      setStarting(false)
    }
  }

  /** Carry on with the stopped install `job` (same answers, licence key kept by Kora): retry the failed step, or skip it. */
  const resume = async (skip?: string, opts: { askAdmin?: boolean; withoutAdmin?: boolean } = {}) => {
    if (!job) return
    setResumeErr(null)
    try {
      const next = (await api.installs.resume(job, skip, opts)).job
      setGithubAsked(false)
      setJob(next)
    } catch (e) {
      setResumeErr(e)
    }
  }
  /** Back to the answers; `section` opens that page (it returns to the summary). */
  const changeAnswers = (section?: string) => {
    setJob(null)
    setResumeErr(null)
    setGithubAsked(false)
    setStep(steps.length - 1)
    setEditing(sectionStep(section, f.mode))
  }
  /** What the buttons under an install error do (see lib/problemActions.tsx). */
  const installActions: ActionHandlers = {
    retry_step: () => resume(),
    skip_step: (a) => resume(a.step),
    retry: () => {
      if (run.error?.resume) return resume()
      setJob(null)
      return start()
    },
    change_answers: (a) => changeAnswers(a.section),
    sign_in_github: () => setGithubAsked(true),
    // Windows said no (or nobody answered): ask again, or carry on as you, keeping what was done.
    request_admin: () => resume(undefined, { askAdmin: true }),
    continue_without_admin: async () => {
      setF(withoutAdmin(f))
      if (run.error?.resume) return resume(undefined, { withoutAdmin: true })
      changeAnswers()
    },
  }
  /** Errors from starting (on the summary): change an answer, or drop the parts that need administrator rights. */
  const startActions: ActionHandlers = {
    change_answers: (a) => { setStartErr(null); setEditing(sectionStep(a.section, f.mode)) },
    continue_without_admin: () => { setStartErr(null); setF(withoutAdmin(f)); setEditing(null) },
    // A remote session: Windows only asks when the person says so.
    request_admin: () => start({ askAdmin: true }),
    sign_in_github: () => { setStartErr(null); setEditing(f.mode === 'development' ? 'github' : null) },
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
                <ErrorNotice error={run.error} title="The install could not finish" actions={installActions} />
                {githubAsked && (
                  <Alert tone="info" title="Sign in to GitHub">
                    <p className="mb-2">When you are signed in, use &quot;Try this step again&quot; above. Your answers are kept.</p>
                    <GithubSignIn onSignedIn={onGithubSignedIn} />
                  </Alert>
                )}
                <ErrorNotice error={resumeErr} actions={installActions} />
                {run.error?.kept && run.error.kept.length > 0 && (
                  <div className="text-sm"><div className="font-semibold">What was kept</div><ul className="list-disc pl-5 text-muted-foreground">{run.error.kept.map((k) => <li key={k}>{k}</li>)}</ul></div>
                )}
                {/* An answer without actions (an older panel, or contact with the job was lost): the plain choices. */}
                {!run.error?.actions?.length && (
                  <div className="flex gap-2">
                    <Button variant="default" onClick={() => { setJob(null); void start() }}>Try again</Button>
                    <Button onClick={() => changeAnswers()}>Change my answers</Button>
                  </div>
                )}
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    )
  }

  const last = at === steps.length - 1
  /** The step on screen: one opened with "Change", or the current one. */
  const showing: StepId = editing ?? id
  const problem = stepProblem(showing, f, ctx)
  // A port check that has not answered yet is a short wait, not something the person must fix.
  const waiting: StepId[] = ports.state === 'checking' && !validatePorts(f.game, f.tx) ? ['ports'] : []
  const toFix = (Object.keys(problems) as StepId[]).filter((s) => !waiting.includes(s))
  const installBlocked = toFix.length > 0
    ? (toFix.length === 1 ? 'The server cannot be installed yet. Fix the problem marked in red above.' : `The server cannot be installed yet. Fix the ${toFix.length} problems marked in red above.`)
    : !diskOk ? 'The server cannot be installed yet. There is not enough free disk space in the server folder. Change the folder above, or free some space.' : null
  const switchFlow = (all: boolean) => { setEveryStep(all); setEditing(null); setStep(0) }
  return (
    <div className="mx-auto max-w-2xl space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="font-heading text-2xl font-bold">Install a new server</h1>
        <Checkbox label="Show advanced options" checked={advanced} onChange={(e) => setAdvanced(e.target.checked)} />
      </div>
      {saved && resumeOffered && at === 0 && !editing && (
        <Alert tone="info" title="Continue where you left off?">
          <p>An earlier install of &quot;{saved.name}&quot; stopped part-way. Kora kept your answers (not your licence key).</p>
          <div className="mt-2 flex gap-2">
            <Button size="sm" onClick={() => { setF(applySaved(f, saved)); setResumeOffered(false); setStep(stepsOf(saved.mode ?? f.mode).indexOf('licence')) }}>Continue where I left off</Button>
            <Button size="sm" onClick={async () => { await api.installs.clearSaved(); setResumeOffered(false); void qc.invalidateQueries({ queryKey: ['install-defaults'] }) }}>Start fresh</Button>
          </div>
        </Alert>
      )}
      <ol className="flex flex-wrap gap-x-4 gap-y-1 text-xs" aria-label="Steps">
        {steps.map((s, i) => (
          <li key={s} aria-current={i === at ? 'step' : undefined} className={cn(i === at ? 'font-semibold text-foreground' : 'text-muted-foreground')}>{i + 1}. {STEP_LABEL[s]}</li>
        ))}
      </ol>
      <Card>
        <CardHeader><CardTitle className="text-lg">{STEP_LABEL[showing]}</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          {showing === 'mode' && <ModeStep {...common} />}
          {showing === 'name' && <NameStep {...common} quick={!everyStep && !editing} />}
          {showing === 'ports' && <PortsStep {...common} ports={ports} />}
          {showing === 'build' && <BuildStep {...common} />}
          {showing === 'resources' && <ResourcesStep {...common} />}
          {showing === 'database' && <DatabaseStep {...common} />}
          {showing === 'options' && <OptionsStep {...common} />}
          {showing === 'licence' && <LicenceStep {...common} />}
          {showing === 'cfx' && <CfxStep {...common} />}
          {showing === 'github' && <GithubStep {...common} signedIn={githubSignedIn} onSignedIn={onGithubSignedIn} />}
          {showing === 'review' && (
            <ReviewStep
              rows={reviewRows(f, req)}
              req={req}
              problems={problems}
              waiting={waiting}
              onChange={(r) => { if (r.advanced) setAdvanced(true); setEditing(r.step) }}
              onDisk={setDiskOk}
              onWithoutAdmin={() => setF(withoutAdmin(f))}
            />
          )}
          <ErrorNotice error={startErr} actions={startActions} />
          {starting && (
            <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" aria-hidden /> Checking your answers (ports, GitHub, permissions) and starting the install. This can take a few seconds.
            </p>
          )}
          {editing ? (
            <>
              {problem && <p className="text-sm">{problem}</p>}
              <div className="flex justify-end pt-2">
                {/* Never disabled: anything still wrong is spelled out on the last screen, beside its line. */}
                <Button variant="default" onClick={() => { setEditing(null); setStep(steps.length - 1) }}>Back to the summary</Button>
              </div>
            </>
          ) : (
            <>
              {problem && !last && <p className="text-sm">{problem}</p>}
              {last && installBlocked && <p role="alert" className="text-sm font-medium text-destructive">{installBlocked}</p>}
              {last && !installBlocked && waiting.length > 0 && <p role="status" className="text-sm">Checking that the ports are free. One moment.</p>}
              <div className="flex justify-between pt-2">
                <Button disabled={at === 0} onClick={() => setStep(at - 1)}>Back</Button>
                {last ? (
                  <Button variant="default" disabled={starting || !!installBlocked || waiting.length > 0} aria-busy={starting} onClick={() => void start()}>
                    {starting ? <><Loader2 className="animate-spin" aria-hidden /> Starting...</> : 'Install the server'}
                  </Button>
                ) : (
                  <Button variant="default" disabled={!!problem} onClick={() => setStep(at + 1)}>Next</Button>
                )}
              </div>
            </>
          )}
        </CardContent>
      </Card>
      {!editing && (everyStep ? (
        <p className="text-sm text-muted-foreground">
          Going through every step. Your answers are kept if you switch.{' '}
          <Button variant="link" className="h-auto p-0" onClick={() => switchFlow(false)}>Use the short setup</Button>
        </p>
      ) : (
        <p className="text-sm text-muted-foreground">
          Kora picks the ports, the database and the rest for you, and shows them before anything is installed.{' '}
          <Button variant="link" className="h-auto p-0" onClick={() => switchFlow(true)}>Choose everything myself</Button>
        </p>
      ))}
    </div>
  )
}
