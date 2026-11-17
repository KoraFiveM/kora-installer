import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Loader2 } from 'lucide-react'
import { api } from '@/api/client'
import type { InstallDefaults, InstallEvent, InstallOutcome, InstallRequest, ProgressEvent, StatusEvent } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { OpenFolderButton, TxAdminButton, useTxAdminLink } from '@/components/ServerLinks'
import { MoreAddresses, ReachCard } from '@/components/ReachCard'
import { CONNECT_NOTE } from '@/lib/serverLinks'
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
import { useInstallBlock } from './install/access'
import { allProblems, applySaved, buildRequest, initialForm, quickStepsFor, sectionStep, STEP_LABEL, stepProblem, stepsFor, withoutAdmin, type StepId, type WizardForm } from './install/form'
import { usePortsCheck } from './install/ports'
import { BuildStep, CfxStep, DatabaseStep, GithubStep, LicenceStep, NameStep, OptionsStep, PortsStep, ResourcesStep, SourceStep, VersionsStep } from './install/Steps'
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
    { label: 'Licence key', value: f.licenceChoice === 'saved' ? (f.licenceName ? `Your saved licence key "${f.licenceName}"` : 'Not picked yet') : f.license.trim() ? (f.saveKey && f.saveKeyName.trim() ? `Provided (hidden). It will be saved as "${f.saveKeyName.trim()}".` : 'Provided (hidden). It will not be saved.') : 'Not given yet', step: 'licence' },
    { label: 'Resources come from', value: f.source === 'clone' ? "Kora's resources as they are (plain copies)" : 'Your own copies on GitHub', step: 'source' },
    { label: 'Versions', value: f.versions === 'released' ? 'Released versions (stable)' : 'Latest changes (the main branch)', step: 'versions' },
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
    value: f.cfxFound ? `${f.cfxFound.username ? `${f.cfxFound.username} (${f.cfxFound.identifier})` : f.cfxFound.identifier}${f.cfxRemembered ? '. Remembered from your last install.' : ''}` : 'Not set. Kora makes a local admin with a generated password.',
    step: 'cfx',
  })
  if (f.source === 'fork') rows.push({ label: 'Your copies go to', value: f.githubOwner || 'Not chosen yet', step: 'github' })
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
  const safeKey = JSON.stringify([req.dir, req.source, req.versions, req.firewall, req.service, req.db, req.txadmin_public])
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
                  : <p role="alert" className="font-medium text-danger-text">{problem}</p>)}
              </dd>
              <dd><Button variant="link" size="sm" className="h-auto p-0 text-sm" aria-label={`Change ${r.label}`} onClick={() => onChange(r)}>Change</Button></dd>
            </div>
          )
        })}
      </dl>
      {disk.data && <Alert tone={disk.data.ok ? 'success' : 'danger'} title="Disk space">{disk.data.message}{!/Gi?B/.test(disk.data.message) && ` ${formatBytes(disk.data.free_bytes)} free.`}</Alert>}
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
  // The same address, hint and note the server's Overview page gives (right for local, tunnelled and remote visits).
  const link = useTxAdminLink(ports.txadmin)
  return (
    <div className="space-y-4">
      <Alert tone={o.running ? 'success' : 'warning'} title={o.running ? 'Your server is installed and running' : 'Your server is installed'}>
        {o.running ? 'Players can connect now.' : o.launch_problem ?? 'It was not started. Start it from the server page.'}
      </Alert>
      {!o.running && o.log_tail.length > 0 && <pre className="max-h-48 overflow-auto rounded-md border border-border bg-ink p-3 font-mono text-xs">{o.log_tail.join('\n')}</pre>}
      <div className="space-y-1.5">
        <div className="text-sm text-muted-foreground">Connect address (players type this in the FiveM console, or use direct connect)</div>
        <CopyField value={o.connect} label="Connect address" />
        <p className="text-xs text-muted-foreground">{CONNECT_NOTE}</p>
      </div>
      <div className="space-y-1.5">
        <div className="text-sm text-muted-foreground">txAdmin</div>
        {link.url ? <CopyField value={link.url} label="txAdmin address" /> : <p className="text-sm">Open it with the button below: it goes through this panel, so no extra port has to be open on this computer.</p>}
        {link.hint && <p className="text-xs text-muted-foreground">{link.hint}</p>}
      </div>
      <ReachCard name={o.name} compact />
      <MoreAddresses name={o.name} />
      {!o.admin_user && cfx && (
        <p className="text-sm">Sign in to txAdmin with your Cfx.re account (<strong>{cfx}</strong>). No password was generated.</p>
      )}
      {o.admin_user && (
        <p className="text-sm">txAdmin administrator: <strong>{o.admin_user}</strong>. Its password was saved in <code className="font-mono text-xs">{o.admin_password_file}</code>. Open that file on this computer to read it, and change the password after you sign in.</p>
      )}
      {o.notes.length > 0 && <ul className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">{o.notes.map((n, i) => <li key={i}>{n}</li>)}</ul>}
      <div className="flex flex-wrap gap-2">
        <TxAdminButton name={o.name} port={ports.txadmin} up={o.running} size="default" />
        <CopyButton text={o.connect} label="Copy connect address" size="default" />
        <OpenFolderButton name={o.name} dir={o.dir} size="default" />
        <Button asChild><Link to={`/servers/${encodeURIComponent(o.name)}`}>Go to the server</Link></Button>
      </div>
    </div>
  )
}

/**
 * Installing is admin-only (remote administrators included). Say so before step 1
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
          {block.reason}
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
  // The saved keys decide which licence answer is pre-selected; if they cannot be read the wizard still opens (paste a key).
  const keys = useQuery({ queryKey: ['keys'], queryFn: api.keys.list, retry: false })
  if (defaults.data && !keys.isPending && !f) setF(initialForm(defaults.data, keys.data?.keys))
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
  /** "Carry on" on an install that stopped earlier did not work (for example: its saved key was deleted). */
  const [carryErr, setCarryErr] = useState<unknown>(null)
  /** A failure offered "Sign in to GitHub": the sign-in is shown under it. */
  const [githubAsked, setGithubAsked] = useState(false)
  /** A fix (installing a tool, signing in, starting the database) worked and the failed step was retried by itself. */
  const [autoRetried, setAutoRetried] = useState(false)
  const arts = useQuery({ queryKey: ['artifacts'], queryFn: api.artifacts })
  const ports = usePortsCheck(f.game, f.tx)
  const run = useJob<InstallOutcome, InstallEvent>(job, onServersChanged)
  /** Windows: the panel can ask Windows for permission itself ("Ask again" / "Ask for administrator permission"). */
  const canAsk = !!useQuery({ queryKey: ['system'], queryFn: api.system, staleTime: 60_000 }).data?.admin_prompt

  const artUrl = arts.data?.items.find((a) => a.build === f.artifactBuild)?.url ?? null
  const req = buildRequest(f, { artifactUrl: artUrl })
  const stepsOf = (source: WizardForm['source']) => (everyStep ? stepsFor(source) : quickStepsFor(source))
  const steps = stepsOf(f.source)
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
      setAutoRetried(false)
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
    setEditing(sectionStep(section, f.source))
  }
  /** A fix launched from the failed step worked: say so and retry that step (only when the install can carry on). */
  const retryAfterFix = run.error?.resume ? async () => { await resume(); setAutoRetried(true) } : undefined
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
    ...(canAsk ? { request_admin: () => resume(undefined, { askAdmin: true }) } : {}),
    continue_without_admin: async () => {
      setF(withoutAdmin(f))
      if (run.error?.resume) return resume(undefined, { withoutAdmin: true })
      changeAnswers()
    },
  }
  /** Errors from starting (on the summary): change an answer, or drop the parts that need administrator rights. */
  const startActions: ActionHandlers = {
    change_answers: (a) => { setStartErr(null); setEditing(sectionStep(a.section, f.source)) },
    continue_without_admin: () => { setStartErr(null); setF(withoutAdmin(f)); setEditing(null) },
    // A remote session: Windows only asks when the person says so.
    ...(canAsk ? { request_admin: () => start({ askAdmin: true }) } : {}),
    sign_in_github: () => { setStartErr(null); setEditing(f.source === 'fork' ? 'github' : null) },
  }

  // ---- running / finished ----
  // Step rows come from the installer; plain status lines come from an install run by the Linux helper.
  const progressRows = run.events.filter((e): e is ProgressEvent => 'step' in e)
  const statusLines = run.events.filter((e): e is StatusEvent => 'type' in e && e.type === 'status')
  if (job) {
    return (
      <div className="mx-auto max-w-2xl space-y-5">
        <h1 className="font-heading text-2xl font-bold">{run.phase === 'done' ? 'All done' : run.phase === 'failed' ? 'The install stopped' : 'Installing your server'}</h1>
        <Card>
          <CardContent className="space-y-5 p-6">
            {autoRetried && run.phase === 'running' && <Alert tone="success" title="Fixed. Trying the step again">That worked, so Kora tried the step that stopped again by itself. You do not need to do anything.</Alert>}
            {run.phase === 'running' && <p className="text-sm text-muted-foreground">This takes a few minutes. You can leave this page open; it keeps going.</p>}
            {(run.phase === 'idle' || run.phase === 'running') && run.events.length === 0 ? (
              <p role="status" className="flex items-center gap-2 text-sm">
                <Loader2 className="size-4 animate-spin text-info" aria-hidden /> Getting ready. The first step shows up in a moment...
              </p>
            ) : (
              <>
                {progressRows.length > 0 && <StepProgress events={progressRows} />}
                {statusLines.length > 0 && (
                  <pre aria-label="What Kora is doing" className="max-h-48 overflow-auto rounded-md border border-border bg-ink p-3 font-mono text-xs">{statusLines.slice(-12).map((l) => l.message).join('\n')}</pre>
                )}
              </>
            )}
            {run.phase === 'done' && run.result && <Outcome o={run.result} ports={req.ports} cfx={req.cfx_identifier} />}
            {run.phase === 'failed' && (
              <div className="space-y-4">
                <ErrorNotice error={run.error} title="The install could not finish" actions={installActions} onFixed={retryAfterFix} />
                {githubAsked && (
                  <Alert tone="info" title="Sign in to GitHub">
                    <p className="mb-2">{retryAfterFix ? 'When you are signed in, Kora tries the step again by itself.' : 'When you are signed in, use "Try this step again" above.'} Your answers are kept.</p>
                    <GithubSignIn onSignedIn={() => { onGithubSignedIn(); void retryAfterFix?.() }} />
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
      {at === 0 && !editing && (
        <p className="text-sm text-muted-foreground">Already have a server on this computer? <Link to="/?add=1" className="text-info underline underline-offset-4">Add an existing server</Link> instead.</p>
      )}
      {saved && !defaults.resumable?.some((r) => r.name === saved.name) && resumeOffered && at === 0 && !editing && (
        <Alert tone="info" title="Continue where you left off?">
          <p>An earlier install of &quot;{saved.name}&quot; stopped part-way. Kora kept your answers{saved.license_key_name ? <> and which saved licence key it used (&quot;{saved.license_key_name}&quot;)</> : ' (not your licence key)'}.</p>
          {defaults.saved_key_missing && <p className="mt-1">The saved licence key &quot;{defaults.saved_key_missing}&quot; is not there any more: you will be asked to pick another.</p>}
          <div className="mt-2 flex gap-2">
            <Button size="sm" onClick={() => { setF(applySaved(f, saved)); setResumeOffered(false); setStep(stepsOf(applySaved(f, saved).source).indexOf('licence')) }}>Continue where I left off</Button>
            <Button size="sm" onClick={async () => { await api.installs.clearSaved(); setResumeOffered(false); void qc.invalidateQueries({ queryKey: ['install-defaults'] }) }}>Start fresh</Button>
          </div>
        </Alert>
      )}
      {defaults.resumable && defaults.resumable.length > 0 && at === 0 && !editing && (
        <Alert tone="info" title="An install stopped and can carry on">
          {defaults.resumable.map((r) => (
            <div key={r.job} className="mt-1 flex flex-wrap items-center gap-2">
              <span>&quot;{r.name}&quot; stopped{r.failed_step ? ` at the step ${r.failed_step}` : ''}. Its answers and licence key are kept.</span>
              <Button size="sm" onClick={async () => { setCarryErr(null); try { setJob((await api.installs.resume(r.job)).job) } catch (e) { setCarryErr(e) } }}>Carry on</Button>
              {saved && saved.name === r.name && (
                <Button size="sm" onClick={() => { setF(applySaved(f, saved)); setResumeOffered(false); setStep(stepsOf(applySaved(f, saved).source).indexOf('licence')) }}>Change my answers first</Button>
              )}
            </div>
          ))}
          <ErrorNotice error={carryErr} actions={startActions} />
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
          {showing === 'source' && <SourceStep {...common} />}
          {showing === 'versions' && <VersionsStep {...common} />}
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
          <ErrorNotice error={startErr} actions={startActions} onFixed={start} />
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
              {last && installBlocked && <p role="alert" className="text-sm font-medium text-danger-text">{installBlocked}</p>}
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
