import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { UninstallItem } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useRole, whyNot } from '@/hooks/useRole'
import { ShortcutsCard } from '@/components/Shortcuts'
import { LicenceKeysCard, RememberedCard } from '@/components/LicenceKeys'
import { RemoteAccessCard, SignedInCard } from '@/components/RemoteAccess'
import { AccountCard } from '@/components/auth/AccountCard'
import { TlsCard } from '@/components/TlsCard'
import { AuditLogCard } from '@/components/AuditLog'
import { JobHistoryCard } from '@/components/JobHistory'
import { SupportBundleLink } from '@/components/SupportBundleLink'
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
import { t, infoOf } from '@/i18n'
import { useLocale } from '@/i18n/react'
import { LanguagePicker } from '@/components/LanguagePicker'
import { rich } from '@/i18n/react'
import '@/i18n/en/core/language'
import '@/i18n/en/settings/settingsPage'

/** `force`: this panel runs in a terminal window and cannot restart the usual way; the restart starts a hidden copy and exits. */
function RestartNow({ version, force = false }: { version?: string; force?: boolean }) {
  const [state, setState] = useState<'idle' | 'restarting' | 'timeout'>('idle')
  const [err, setErr] = useState<unknown>(null)
  const go = async () => {
    setErr(null); setState('restarting')
    try {
      if (await restartPanel(undefined, undefined, force)) window.location.reload()
      else setState('timeout')
    } catch (e) { setErr(e); setState('idle') }
  }
  return (
    <div className="space-y-3">
      {force
        ? <Alert tone="success">{rich('settings.settingsPage.updatedPanelRunsTerminal', { code: (c) => <code>{c}</code> })}</Alert>
        : <Alert tone="success">{version ? t('settings.settingsPage.updatedTo', { version }) : t('settings.settingsPage.updatedValue')}</Alert>}
      {force && <p className="text-sm text-muted-foreground">{rich('settings.settingsPage.forceRestartNote', { code: (c) => <code>{c}</code> })}</p>}
      {state === 'restarting' ? <p className="text-sm">{t('settings.settingsPage.restarting')}</p> : <Button onClick={() => void go()}>{t(force ? 'settings.settingsPage.forceRestart' : 'settings.settingsPage.restartNow')}</Button>}
      {state === 'timeout' && <Alert tone="warning">{rich('settings.settingsPage.panelDidComeBack', { code: (c) => <code>{c}</code> })}</Alert>}
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
      <CardHeader><CardTitle>{t('settings.settingsPage.koraVersion')}</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap items-center gap-3 text-sm">
          <span>{rich('settings.settingsPage.installedStrongNode', { strong: <strong>{q.data?.current ?? '...'}</strong> })}</span>
          {q.data && (q.data.available ? <Badge tone="info">{t('settings.settingsPage.versionLatestAvailable', { latest: q.data.latest ?? '' })}</Badge> : <Badge tone="plain">{t('settings.settingsPage.upDate')}</Badge>)}
          <Button size="sm" onClick={() => void q.refetch()} disabled={q.isFetching}>{t('settings.settingsPage.checkAgain')}</Button>
        </div>
        <div className="flex flex-wrap items-end gap-3">
          <Button variant={q.data?.available ? 'default' : 'secondary'} disabled={run.phase === 'running' || !q.data?.latest || blocked !== null} onClick={() => go()}>{t('settings.settingsPage.updateLatest')}</Button>
          <div className="w-48"><Field label={t('settings.settingsPage.specificVersion')}><Select value={version} onChange={(e) => setVersion(e.target.value)}><option value="">{t('settings.settingsPage.choose')}</option>{(q.data?.versions ?? []).map((v) => <option key={v}>{v}</option>)}</Select></Field></div>
          <Button disabled={!version || run.phase === 'running' || blocked !== null} onClick={() => go(version)}>{t('settings.settingsPage.installVersion')}</Button>
        </div>
        {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
        {run.phase !== 'idle' && <ResourceProgress events={run.events} />}
        {run.phase === 'done' && (sys.data?.can_restart
          ? <RestartNow version={q.data?.current} />
          : <RestartNow version={q.data?.current} force />)}
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
      <CardHeader><CardTitle>{t('settings.settingsPage.uninstallKora')}</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">{t('settings.settingsPage.removesWhatKoraAdded')}</p>
        {!role.canDestroy && <p className="text-xs text-muted-foreground">{whyNot(role, 'admin', true)}</p>}
        {!items && <Button onClick={() => plan.mutate()} disabled={plan.isPending || !role.canDestroy}>{t('settings.settingsPage.showWhatWouldRemoved')}</Button>}
        {items && !job && (
          <div className="space-y-4">
            <fieldset className="space-y-3">
              <legend className="mb-1 text-sm font-semibold">{t('settings.settingsPage.removeThese')}</legend>
              {items.map((i, idx) => (
                <Checkbox key={i.item} label={i.description} hint={i.kind === 'folder' || i.kind === 'database' ? t('settings.settingsPage.deletesDataCannotUndone') : undefined} checked={i.remove} onChange={(e) => setItems(items.map((x, k) => (k === idx ? { ...x, remove: e.target.checked } : x)))} />
              ))}
            </fieldset>
            {danger && <Alert tone="danger">{t('settings.settingsPage.choseDeleteServerFolders')}</Alert>}
            <Field label={t('settings.settingsPage.typeUninstallConfirm')}><Input value={typed} onChange={(e) => setTyped(e.target.value)} autoComplete="off" /></Field>
            <Button variant="destructive" disabled={typed !== 'uninstall' || start.isPending} onClick={() => start.mutate()}>{t('settings.settingsPage.uninstall')}</Button>
          </div>
        )}
        {job && <ResourceProgress events={run.events} />}
        {run.phase === 'done' && <Alert tone="success" title={t('settings.settingsPage.uninstallFinished')}>{run.result?.notes?.join(' ')}</Alert>}
        {run.phase === 'failed' && <ErrorNotice error={run.error} />}
        <ErrorNotice error={plan.error ?? start.error} />
      </CardContent>
    </Card>
  )
}

/** Settings > Language: the picker, remembered by this browser. */
function LanguageCard() {
  const current = useLocale()
  const beta = infoOf(current).beta
  return (
    <Card>
      <CardHeader><CardTitle>{t('core.language.label')}</CardTitle></CardHeader>
      <CardContent className="space-y-2">
        <LanguagePicker />
        <p className="text-xs text-muted-foreground">{beta ? t('core.language.hint') : t('core.language.remembered')}</p>
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
      <h1 className="font-heading text-2xl font-bold">{t('settings.settingsPage.settings')}</h1>
      <LanguageCard />
      {role.remote ? <SignedInCard /> : <RemoteAccessCard />}
      {role.remote && <AccountCard />}
      {role.can('admin') && <TlsCard />}
      <SelfUpdate />
      <ShortcutsCard />
      <LicenceKeysCard />
      <RememberedCard />
      <JobHistoryCard />
      <AuditLogCard />
      <Card><CardHeader><CardTitle>{t('settings.settingsPage.githubAccount')}</CardTitle></CardHeader><CardContent><GithubSignIn /></CardContent></Card>
      <Card>
        <CardHeader><CardTitle>{t('settings.settingsPage.getHelp')}</CardTitle></CardHeader>
        <CardContent className="space-y-3 text-sm">
          <p>{t('settings.settingsPage.whenSomethingGoesWrong')}</p>
          {role.can('admin') ? <SupportBundleLink label={t('settings.settingsPage.createSupportBundle')} /> : <p className="text-muted-foreground">{t('settings.settingsPage.askAdministratorMakeOne')}</p>}
        </CardContent>
      </Card>
      <Uninstall />
      <Card>
        <CardHeader><CardTitle>{t('settings.settingsPage.about')}</CardTitle></CardHeader>
        <CardContent className="space-y-2 text-sm">
          <p>{t(sys.data?.admin_service?.state === 'ready' ? 'settings.settingsPage.koraPanelVersionManagesAdmin' : 'settings.settingsPage.koraPanelVersionManages', { version: meta.data?.version ?? '', os: meta.data?.platform === 'windows' ? 'Windows' : 'Linux' })}</p>
          <p className="text-muted-foreground">{t('settings.settingsPage.websiteKoraframeWork')}</p>
          <p className="break-all text-muted-foreground">{t('settings.settingsPage.documentationDocsKoraframeWork')}</p>
          <p className="break-all text-muted-foreground">{rich('settings.settingsPage.sourceReleasesLinkGithub', { link: (c) => <a className="text-info underline" href="https://github.com/KoraFiveM/kora-installer" target="_blank" rel="noopener noreferrer">{c}</a> })}</p>
          <p className="text-muted-foreground">{t('settings.settingsPage.pageMakesNoRequests')}</p>
        </CardContent>
      </Card>
    </div>
  )
}
