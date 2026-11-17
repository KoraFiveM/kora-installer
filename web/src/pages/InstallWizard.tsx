import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { BackupSuggestion } from '@/components/BackupSuggestion'
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
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/install'

interface ReviewRow {
  label: string
  value: string
  /** The step that "Change" opens. */
  step: StepId
  /** The answer only shows in that step with the advanced options on. */
  advanced?: boolean
}

function reviewRows(f: WizardForm, req: InstallRequest): ReviewRow[] {
  const build = f.artifact === 'recommended' ? 'Recommended' : f.artifact === 'pick' ? (f.artifactBuild === null ? t('install.installWizard.pickedYet') : `Build ${f.artifactBuild}`) : t('install.installWizard.customDownload')
  const rows: ReviewRow[] = [
    { label: t('install.installWizard.licenceKey'), value: f.licenceChoice === 'saved' ? (f.licenceName ? t('install.installWizard.savedLicenceKeyLicencename', { licenceName: f.licenceName }) : t('install.installWizard.pickedYet')) : f.license.trim() ? (f.saveKey && f.saveKeyName.trim() ? t('install.installWizard.providedHiddenSavedTrim', { trim: f.saveKeyName.trim() }) : t('install.installWizard.providedHiddenSaved')) : t('install.installWizard.givenYet'), step: 'licence' },
    { label: t('install.installWizard.resourcesComeFrom'), value: f.source === 'clone' ? t('install.installWizard.korasResourcesTheyPlain') : t('install.installWizard.ownCopiesGithub'), step: 'source' },
    { label: 'Versions', value: f.versions === 'released' ? t('install.installWizard.releasedVersionsStable') : t('install.installWizard.latestChangesMainBranch'), step: 'versions' },
    { label: 'Name', value: req.name || t('install.installWizard.givenYet'), step: 'name' },
    { label: 'Folder', value: req.dir || t('install.installWizard.givenYet'), step: 'name' },
    { label: t('install.installWizard.serverNamePlayers'), value: req.hostname || '(default)', step: 'name', advanced: true },
    { label: 'Ports', value: t('install.installWizard.gameValueTxadminValue', { value: f.game.trim() || '?', value2: f.tx.trim() || '?' }), step: 'ports' },
    { label: t('install.installWizard.fivemServerBuild'), value: f.gameBuild.trim() ? t('install.installWizard.buildGameBuildTrim', { build, trim: f.gameBuild.trim() }) : build, step: 'build' },
    { label: 'Resources', value: f.resourcesAll ? 'Everything' : `${f.resourceIds.length} picked, plus what they need`, step: 'resources' },
    { label: 'Database', value: f.db === 'use_existing' ? t('install.installWizard.useExistingDatabase') : f.db === 'install_if_missing' ? t('install.installWizard.setUpMe') : 'Skip', step: 'database' },
    { label: t('install.installWizard.databaseLogin'), value: f.db === 'skip' ? t('install.installWizard.notNeeded') : f.dbCustom ? t('install.installWizard.typedUserDbuserPassword', { dbUser: f.dbUser }) : 'Automatic', step: 'database' },
    { label: 'Firewall', value: req.firewall ? (req.txadmin_public ? t('install.installWizard.openGameTxadminPorts') : t('install.installWizard.openGamePort')) : t('install.installWizard.leaveClosed'), step: 'options' },
    { label: t('install.installWizard.backgroundService'), value: req.service ? 'Yes' : 'No', step: 'options' },
  ]
  rows.push({
    label: t('install.installWizard.signTxadminCfxRe'),
    value: f.cfxFound ? `${f.cfxFound.username ? `${f.cfxFound.username} (${f.cfxFound.identifier})` : f.cfxFound.identifier}${f.cfxRemembered ? t('install.installWizard.rememberedFromLastInstall') : ''}` : t('install.installWizard.setKoraMakesLocal'),
    step: 'cfx',
  })
  if (f.source === 'fork') rows.push({ label: t('install.installWizard.copiesGo'), value: f.githubOwner || t('install.installWizard.chosenYet'), step: 'github' })
  return rows
}

/** The administrator service: what it does for this install (ready), or that it is not there (a per-user install). */
export function AdminServiceNotice({ state, needs, onWithout }: { state: 'ready' | 'missing' | 'not_allowed'; needs: string[]; onWithout?: () => void }) {
  if (needs.length === 0) return null
  const list = <ul className="mt-1 list-disc pl-5">{needs.map((n) => <li key={n}>{n}</li>)}</ul>
  if (state === 'ready') {
    return (
      <Alert tone="info" title={t('install.installWizard.korasAdministratorServiceDo')}>
        <p>{t('install.installWizard.thesePartsNeedAdministrator')}</p>
        {list}
      </Alert>
    )
  }
  return (
    <Alert tone="warning" title={state === 'missing' ? t('install.installWizard.korasAdministratorServiceInsta') : t('install.installWizard.korasAdministratorServiceTurne')}>
      <p>{t('install.installWizard.thesePartsNeedAdministrator2')}</p>
      {list}
      <p className="mt-2">
        {state === 'missing'
          ? t('install.installWizard.installAdministratorServiceOnc')
          : t('install.installWizard.administratorTurnServiceAccoun')}
      </p>
      {onWithout && (
        <p className="mt-2">{rich('install.installWizard.buttonInstallWithoutThese', { button: (c) => <Button variant="link" className="h-auto p-0" onClick={onWithout}>{c}</Button> })}</p>
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
      <p className="text-sm text-muted-foreground">{t('install.installWizard.whatKoraDoSuggested')}</p>
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
              <dd><Button variant="link" size="sm" className="h-auto p-0 text-sm" aria-label={t('install.installWizard.changeLabel', { label: r.label })} onClick={() => onChange(r)}>{t('install.installWizard.change')}</Button></dd>
            </div>
          )
        })}
      </dl>
      {disk.data && <Alert tone={disk.data.ok ? 'success' : 'danger'} title={t('install.installWizard.diskSpace')}>{disk.data.message}{!/Gi?B/.test(disk.data.message) && t('install.installWizard.formatbytesFree', { formatBytes: formatBytes(disk.data.free_bytes) })}</Alert>}
      <ErrorNotice error={disk.error ?? plan.error} />
      {plan.data && plan.data.admin_service && plan.data.needs_admin.length > 0 && <AdminServiceNotice state={plan.data.admin_service} needs={plan.data.needs_admin} onWithout={onWithoutAdmin} />}
      {plan.data && !(plan.data.admin_service && plan.data.needs_admin.length > 0) && (
        <Alert tone={plan.data.needs_admin.length ? 'warning' : 'info'} title={t('install.installWizard.administratorRights')}>
          {plan.data.needs_admin.length === 0 ? t('install.installWizard.nothingInstallNeedsAdministrat') : (
            <>
              <p>{t('install.installWizard.theseStepsNeedAdministrator')}</p>
              <ul className="mt-1 list-disc pl-5">{plan.data.needs_admin.map((n) => <li key={n}>{n}</li>)}</ul>
            </>
          )}
        </Alert>
      )}
      <p className="text-xs text-muted-foreground">{t('install.installWizard.valueLicenceKeyDatabase', { value: plan.data ? `${plan.data.steps.length} steps will run.` : '' })}</p>
    </div>
  )
}

export function Outcome({ o, ports, cfx }: { o: InstallOutcome; ports: { txadmin: number }; cfx?: string | null }) {
  // The same address, hint and note the server's Overview page gives (right for local, tunnelled and remote visits).
  const link = useTxAdminLink(ports.txadmin)
  return (
    <div className="space-y-4">
      <Alert tone={o.running ? 'success' : 'warning'} title={o.running ? t('install.installWizard.serverInstalledRunning') : t('install.installWizard.serverInstalled')}>
        {o.running ? t('install.installWizard.playersConnectNow') : o.launch_problem ?? t('install.installWizard.startedStartFromServer')}
      </Alert>
      {!o.running && o.log_tail.length > 0 && <pre className="max-h-48 overflow-auto rounded-md border border-border bg-ink p-3 font-mono text-xs">{o.log_tail.join('\n')}</pre>}
      <div className="space-y-1.5">
        <div className="text-sm text-muted-foreground">{t('install.installWizard.connectAddressPlayersType')}</div>
        <CopyField value={o.connect} label={t('install.installWizard.connectAddress')} />
        <p className="text-xs text-muted-foreground">{t(CONNECT_NOTE)}</p>
      </div>
      <div className="space-y-1.5">
        <div className="text-sm text-muted-foreground">txAdmin</div>
        {link.url ? <CopyField value={link.url} label={t('install.installWizard.txadminAddress')} /> : <p className="text-sm">{t('install.installWizard.openButtonBelowGoes')}</p>}
        {link.hint && <p className="text-xs text-muted-foreground">{link.hint}</p>}
      </div>
      <ReachCard name={o.name} compact />
      <MoreAddresses name={o.name} />
      {!o.admin_user && cfx && (
        <p className="text-sm">{rich('install.installWizard.signTxadminCfxRe2', { strong: <strong>{cfx}</strong> })}</p>
      )}
      {o.admin_user && (
        <p className="text-sm">{rich('install.installWizard.txadminAdministratorStrongNode', { strong: <strong>{o.admin_user}</strong>, code: <code className="font-mono text-xs">{o.admin_password_file}</code> })}</p>
      )}
      {o.notes.length > 0 && <ul className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">{o.notes.map((n, i) => <li key={i}>{n}</li>)}</ul>}
      <div className="flex flex-wrap gap-2">
        <TxAdminButton name={o.name} port={ports.txadmin} up={o.running} size="default" />
        <CopyButton text={o.connect} label={t('install.installWizard.copyConnectAddress')} size="default" />
        <OpenFolderButton name={o.name} dir={o.dir} size="default" />
        <Button asChild><Link to={`/servers/${encodeURIComponent(o.name)}`}>{t('install.installWizard.goServer')}</Link></Button>
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
        <h1 className="font-heading text-2xl font-bold">{t('install.installWizard.installNewServer')}</h1>
        <Alert tone="warning" title={t('install.installWizard.cannotInstallServerFrom')}>
          {block.reason}
        </Alert>
        <Button asChild><Link to="/">{t('install.installWizard.backServers')}</Link></Button>
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

  /** Carry on with the stopped install `job` (same answers, licence key kept by Kora): retry the failed step, or skip it. */
  const resume = async (skip?: string, opts: { withoutAdmin?: boolean } = {}) => {
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
    // The service stopped midway: carry on as you without the administrator parts, keeping what was done.
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
    sign_in_github: () => { setStartErr(null); setEditing(f.source === 'fork' ? 'github' : null) },
  }

  // ---- running / finished ----
  // Step rows come from the installer; plain status lines come from an install run by the Linux helper.
  const progressRows = run.events.filter((e): e is ProgressEvent => 'step' in e)
  const statusLines = run.events.filter((e): e is StatusEvent => 'type' in e && e.type === 'status')
  if (job) {
    return (
      <div className="mx-auto max-w-2xl space-y-5">
        <h1 className="font-heading text-2xl font-bold">{run.phase === 'done' ? t('install.installWizard.allDone') : run.phase === 'failed' ? t('install.installWizard.installStopped') : t('install.installWizard.installingServer')}</h1>
        <Card>
          <CardContent className="space-y-5 p-6">
            {autoRetried && run.phase === 'running' && <Alert tone="success" title={t('install.installWizard.fixedTryingStepAgain')}>{t('install.installWizard.workedKoraTriedStep')}</Alert>}
            {run.phase === 'running' && <p className="text-sm text-muted-foreground">{t('install.installWizard.takesFewMinutesLeave')}</p>}
            {(run.phase === 'idle' || run.phase === 'running') && run.events.length === 0 ? (
              <p role="status" className="flex items-center gap-2 text-sm">{rich('install.installWizard.loaderNodeGettingReady', { loader2: <Loader2 className="size-4 animate-spin text-info" aria-hidden /> })}</p>
            ) : (
              <>
                {progressRows.length > 0 && <StepProgress events={progressRows} />}
                {statusLines.length > 0 && (
                  <pre aria-label={t('install.installWizard.whatKoraDoing')} className="max-h-48 overflow-auto rounded-md border border-border bg-ink p-3 font-mono text-xs">{statusLines.slice(-12).map((l) => l.message).join('\n')}</pre>
                )}
              </>
            )}
            {run.phase === 'done' && run.result && <Outcome o={run.result} ports={req.ports} cfx={req.cfx_identifier} />}
            {run.phase === 'done' && run.result?.name && <BackupSuggestion server={run.result.name} compact />}
            {run.phase === 'failed' && (
              <div className="space-y-4">
                <ErrorNotice error={run.error} title={t('install.installWizard.installCouldFinish')} actions={installActions} onFixed={retryAfterFix} />
                {githubAsked && (
                  <Alert tone="info" title={t('install.installWizard.signGithub')}>
                    <p className="mb-2">{t('install.installWizard.valueAnswersKept', { value: retryAfterFix ? t('install.installWizard.whenSignedKoraTries') : t('install.installWizard.whenSignedUseTry') })}</p>
                    <GithubSignIn onSignedIn={() => { onGithubSignedIn(); void retryAfterFix?.() }} />
                  </Alert>
                )}
                <ErrorNotice error={resumeErr} actions={installActions} />
                {run.error?.kept && run.error.kept.length > 0 && (
                  <div className="text-sm"><div className="font-semibold">{t('install.installWizard.whatKept')}</div><ul className="list-disc pl-5 text-muted-foreground">{run.error.kept.map((k) => <li key={k}>{k}</li>)}</ul></div>
                )}
                {/* An answer without actions (an older panel, or contact with the job was lost): the plain choices. */}
                {!run.error?.actions?.length && (
                  <div className="flex gap-2">
                    <Button variant="default" onClick={() => { setJob(null); void start() }}>{t('install.installWizard.tryAgain')}</Button>
                    <Button onClick={() => changeAnswers()}>{t('install.installWizard.changeMyAnswers')}</Button>
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
    ? (toFix.length === 1 ? t('install.installWizard.serverCannotInstalledYet') : t('install.installWizard.serverCannotInstalledYet2', { count: toFix.length }))
    : !diskOk ? t('install.installWizard.serverCannotInstalledYet3') : null
  const switchFlow = (all: boolean) => { setEveryStep(all); setEditing(null); setStep(0) }
  return (
    <div className="mx-auto max-w-2xl space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="font-heading text-2xl font-bold">{t('install.installWizard.installNewServer')}</h1>
        <Checkbox label={t('install.installWizard.showAdvancedOptions')} checked={advanced} onChange={(e) => setAdvanced(e.target.checked)} />
      </div>
      {at === 0 && !editing && (
        <p className="text-sm text-muted-foreground">{rich('install.installWizard.alreadyServerComputerLink', { link: (c) => <Link to="/?add=1" className="text-info underline underline-offset-4">{c}</Link> })}</p>
      )}
      {at === 0 && !editing && (
        <p className="text-sm text-muted-foreground">{rich('install.installWizard.movingServerFromAnother', { link: (c) => <Link to="/import" className="text-info underline underline-offset-4">{c}</Link> })}</p>
      )}
      {saved && !defaults.resumable?.some((r) => r.name === saved.name) && resumeOffered && at === 0 && !editing && (
        <Alert tone="info" title={t('install.installWizard.continueWhereLeftOff')}>
          <p>{rich('install.installWizard.earlierInstallNameStopped', { name: saved.name, node: saved.license_key_name ? <>{t('install.installWizard.whichSavedLicenceKey', { license_key_name: saved.license_key_name })}</> : t('install.installWizard.licenceKey2') })}</p>
          {defaults.saved_key_missing && <p className="mt-1">{t('install.installWizard.savedLicenceKeySaved', { saved_key_missing: defaults.saved_key_missing })}</p>}
          <div className="mt-2 flex gap-2">
            <Button size="sm" onClick={() => { setF(applySaved(f, saved)); setResumeOffered(false); setStep(stepsOf(applySaved(f, saved).source).indexOf('licence')) }}>{t('install.installWizard.continueWhereILeft')}</Button>
            <Button size="sm" onClick={async () => { await api.installs.clearSaved(); setResumeOffered(false); void qc.invalidateQueries({ queryKey: ['install-defaults'] }) }}>{t('install.installWizard.startFresh')}</Button>
          </div>
        </Alert>
      )}
      {defaults.resumable && defaults.resumable.length > 0 && at === 0 && !editing && (
        <Alert tone="info" title={t('install.installWizard.installStoppedCarry')}>
          {defaults.resumable.map((r) => (
            <div key={r.job} className="mt-1 flex flex-wrap items-center gap-2">
              <span>{t('install.installWizard.nameStoppedValueIts', { name: r.name, value: r.failed_step ? ` at the step ${r.failed_step}` : '' })}</span>
              <Button size="sm" onClick={async () => { setCarryErr(null); try { setJob((await api.installs.resume(r.job)).job) } catch (e) { setCarryErr(e) } }}>{t('install.installWizard.carryOn')}</Button>
              {saved && saved.name === r.name && (
                <Button size="sm" onClick={() => { setF(applySaved(f, saved)); setResumeOffered(false); setStep(stepsOf(applySaved(f, saved).source).indexOf('licence')) }}>{t('install.installWizard.changeMyAnswersFirst')}</Button>
              )}
            </div>
          ))}
          <ErrorNotice error={carryErr} actions={startActions} />
        </Alert>
      )}
      <ol className="flex flex-wrap gap-x-4 gap-y-1 text-xs" aria-label={t('install.installWizard.steps')}>
        {steps.map((s, i) => (
          <li key={s} aria-current={i === at ? 'step' : undefined} className={cn(i === at ? 'font-semibold text-foreground' : 'text-muted-foreground')}>{i + 1}. {t(STEP_LABEL[s])}</li>
        ))}
      </ol>
      <Card>
        <CardHeader><CardTitle className="text-lg">{t(STEP_LABEL[showing])}</CardTitle></CardHeader>
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
            <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">{rich('install.installWizard.loaderNodeCheckingAnswers', { loader2: <Loader2 className="size-4 animate-spin" aria-hidden /> })}</p>
          )}
          {editing ? (
            <>
              {problem && <p className="text-sm">{problem}</p>}
              <div className="flex justify-end pt-2">
                {/* Never disabled: anything still wrong is spelled out on the last screen, beside its line. */}
                <Button variant="default" onClick={() => { setEditing(null); setStep(steps.length - 1) }}>{t('install.installWizard.backSummary')}</Button>
              </div>
            </>
          ) : (
            <>
              {problem && !last && <p className="text-sm">{problem}</p>}
              {last && installBlocked && <p role="alert" className="text-sm font-medium text-danger-text">{installBlocked}</p>}
              {last && !installBlocked && waiting.length > 0 && <p role="status" className="text-sm">{t('install.installWizard.checkingPortsFreeOne')}</p>}
              <div className="flex justify-between pt-2">
                <Button disabled={at === 0} onClick={() => setStep(at - 1)}>{t('install.installWizard.back')}</Button>
                {last ? (
                  <Button variant="default" disabled={starting || !!installBlocked || waiting.length > 0} aria-busy={starting} onClick={() => void start()}>
                    {starting ? <>{rich('install.installWizard.loaderNodeStarting', { loader2: <Loader2 className="animate-spin" aria-hidden /> })}</> : t('install.installWizard.installServer')}
                  </Button>
                ) : (
                  <Button variant="default" disabled={!!problem} onClick={() => setStep(at + 1)}>{t('install.installWizard.next')}</Button>
                )}
              </div>
            </>
          )}
        </CardContent>
      </Card>
      {!editing && (everyStep ? (
        <p className="text-sm text-muted-foreground">{rich('install.installWizard.goingThroughEveryStep', { button: (c) => <Button variant="link" className="h-auto p-0" onClick={() => switchFlow(false)}>{c}</Button> })}</p>
      ) : (
        <p className="text-sm text-muted-foreground">{rich('install.installWizard.koraPicksPortsDatabase', { button: (c) => <Button variant="link" className="h-auto p-0" onClick={() => switchFlow(true)}>{c}</Button> })}</p>
      ))}
    </div>
  )
}
