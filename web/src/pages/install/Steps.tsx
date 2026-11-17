import { useEffect, useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ExternalLink, Eye, EyeOff, Loader2 } from 'lucide-react'
import { api } from '@/api/client'
import type { DbChoice, Problem, Source } from '@/api/types'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox, Radio } from '@/components/ui/checkbox'
import { Input, Label, Select } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { GithubSignIn } from '@/components/GithubSignIn'
import { GROUP_LABEL, groupResources } from '@/lib/catalog'
import { slug, validatePorts } from '@/lib/utils'
import { dbLoginProblem, defaultVersions, dirBase, folderFor, type WizardForm } from './form'
import type { PortsChecked } from './ports'
import { t, msg, type Msg } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/install/steps'

export interface StepProps {
  f: WizardForm
  set: (patch: Partial<WizardForm>) => void
  advanced: boolean
  defaultsDir: string
}

/** Choosing the source also moves "Which versions" to its usual answer, until the person has chosen versions themselves. */
export function sourcePatch(f: WizardForm, source: Source): Partial<WizardForm> {
  return { source, ...(f.versionsTouched ? {} : { versions: defaultVersions(source) }) }
}

export function SourceStep({ f, set }: StepProps) {
  return (
    <fieldset className="space-y-3">
      <legend className="mb-2 text-sm text-muted-foreground">{t('install.steps.whereShouldResourcesCome')}</legend>
      <Radio name="source" checked={f.source === 'clone'} onChange={() => set(sourcePatch(f, 'clone'))} label={t('install.steps.useKorasResourcesThey')} hint={t('install.steps.plainCopiesClonesOriginal')} />
      <Radio name="source" checked={f.source === 'fork'} onChange={() => set(sourcePatch(f, 'fork'))} label={t('install.steps.makeMyOwnCopies')} hint={t('install.steps.koraForksEachResource')} />
    </fieldset>
  )
}

export function VersionsStep({ f, set }: StepProps) {
  return (
    <fieldset className="space-y-3">
      <legend className="mb-2 text-sm text-muted-foreground">{t('install.steps.whichVersionsShouldServer')}</legend>
      <Radio name="versions" checked={f.versions === 'released'} onChange={() => set({ versions: 'released', versionsTouched: true })} label={t('install.steps.releasedVersionsRecommendedSta')} hint={t('install.steps.everyResourcePinnedTested')} />
      <Radio name="versions" checked={f.versions === 'latest'} onChange={() => set({ versions: 'latest', versionsTouched: true })} label={t('install.steps.latestChangesMainBranch')} hint={t('install.steps.everyResourceFollowsNewest')} />
      <p className="text-xs text-muted-foreground">{t('install.steps.changeLaterServersSettings')}</p>
    </fieldset>
  )
}

export function NameStep({ f, set, advanced, defaultsDir, quick }: StepProps & { /** The short path: the folder and the kind of server are shown as plain text until asked for. */ quick?: boolean }) {
  const base = dirBase(defaultsDir)
  const [folderOpen, setFolderOpen] = useState(false)
  const [kindOpen, setKindOpen] = useState(false)
  const showFolder = !quick || advanced || folderOpen || f.dirTouched
  const showKind = kindOpen || f.source === 'fork'
  return (
    <div className="space-y-4">
      <Field label={t('install.steps.serverName')} hint={t('install.steps.usedFolderServiceDatabase', { name: slug(f.name) || '...' })}>
        <Input value={f.name} onChange={(e) => set({ name: e.target.value, ...(f.dirTouched ? {} : { dir: folderFor(base, e.target.value) }) })} autoComplete="off" />
      </Field>
      {showFolder ? (
        <Field label={t('install.steps.serverFolder')} hint={t('install.steps.whereServerFilesGo')}>
          <Input value={f.dir} onChange={(e) => set({ dir: e.target.value, dirTouched: true })} autoComplete="off" />
        </Field>
      ) : (
        <p className="text-sm text-muted-foreground">{rich('install.steps.serverFilesGoCode', { code: <code className="break-all font-mono text-xs text-foreground">{f.dir}</code>, button: (c) => <Button variant="link" className="h-auto p-0" onClick={() => setFolderOpen(true)}>{c}</Button> })}</p>
      )}
      {advanced && (
        <Field label={t('install.steps.serverNameShownPlayers')} hint={t('install.steps.shownFivemServerList')}>
          <Input value={f.hostname} onChange={(e) => set({ hostname: e.target.value })} />
        </Field>
      )}
      {quick && (showKind ? (
        <div className="space-y-4">
          <SourceStep f={f} set={set} advanced={advanced} defaultsDir={defaultsDir} />
          <VersionsStep f={f} set={set} advanced={advanced} defaultsDir={defaultsDir} />
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">{rich('install.steps.resourcesPlainCopiesOriginals', { button: (c) => <Button variant="link" className="h-auto p-0" onClick={() => setKindOpen(true)}>{c}</Button> })}</p>
      ))}
    </div>
  )
}

export function PortsStep({ f, set, ports }: StepProps & { ports: PortsChecked }) {
  const { check } = ports
  const problem = validatePorts(f.game, f.tx)
  const line = (label: string, key: 'game' | 'txadmin', field: 'game' | 'tx') => {
    const l = check?.[key]
    return (
      <div className="space-y-1.5">
        <Field label={label}><Input inputMode="numeric" value={f[field]} onChange={(e) => set({ [field]: e.target.value } as Partial<WizardForm>)} /></Field>
        {l && (
          <div className="flex flex-wrap items-center gap-2 text-sm" aria-live="polite">
            <span className={l.ok ? 'text-success' : 'text-danger-text'}>{l.text}</span>
            {l.suggestion && <Button size="sm" onClick={() => set({ [field]: String(l.suggestion) } as Partial<WizardForm>)}>{t('install.steps.useSuggestion', { suggestion: l.suggestion })}</Button>}
          </div>
        )}
      </div>
    )
  }
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">{t('install.steps.playersConnectGamePort')}</p>
      <div className="grid gap-4 sm:grid-cols-2">
        {line(t('install.steps.gamePort'), 'game', 'game')}
        {line(t('install.steps.txadminPort'), 'txadmin', 'tx')}
      </div>
      {problem && <p role="alert" className="text-sm text-danger-text">{problem}</p>}
      <ErrorNotice error={ports.error} />
      {ports.state === 'unknown' && <Button onClick={ports.retry}>{t('install.steps.checkPortsAgain')}</Button>}
    </div>
  )
}

export function BuildStep({ f, set, advanced }: StepProps) {
  const arts = useQuery({ queryKey: ['artifacts'], queryFn: api.artifacts })
  return (
    <div className="space-y-4">
      <fieldset className="space-y-3">
        <legend className="mb-2 text-sm text-muted-foreground">{t('install.steps.whichVersionFivemServer')}</legend>
        <Radio name="art" checked={f.artifact === 'recommended'} onChange={() => set({ artifact: 'recommended' })} label={t('install.steps.recommendedBuild')} hint={arts.data?.recommended ? t('install.steps.buildRecommendedOneCfx', { recommended: arts.data.recommended }) : t('install.steps.oneCfxReSuggests')} />
        <Radio name="art" checked={f.artifact === 'pick'} onChange={() => set({ artifact: 'pick' })} label={t('install.steps.pickBuildFromList')} />
        {f.artifact === 'pick' && (
          <Field label={t('install.steps.build')}>
            <Select value={f.artifactBuild ?? ''} onChange={(e) => set({ artifactBuild: e.target.value ? Number(e.target.value) : null })}>
              <option value="">{t('install.steps.chooseBuild')}</option>
              {(arts.data?.items ?? []).map((a) => <option key={a.build} value={a.build}>{a.build}{a.build === arts.data?.recommended ? t('install.steps.recommended') : ''}</option>)}
            </Select>
          </Field>
        )}
        <Radio name="art" checked={f.artifact === 'custom'} onChange={() => set({ artifact: 'custom' })} label={t('install.steps.useMyOwnDownload')} />
        {f.artifact === 'custom' && <Field label={t('install.steps.downloadAddress')} hint={t('install.steps.linkServerZipWindows')}><Input value={f.artifactUrl} onChange={(e) => set({ artifactUrl: e.target.value })} placeholder={t('install.steps.https')} /></Field>}
      </fieldset>
      <ErrorNotice error={arts.error} />
      {advanced && <Field label={t('install.steps.gameBuildOptional')} hint={t('install.steps.forcesGtaVGame')}><Input inputMode="numeric" value={f.gameBuild} onChange={(e) => set({ gameBuild: e.target.value })} /></Field>}
    </div>
  )
}

export function ResourcesStep({ f, set }: StepProps) {
  const cat = useQuery({ queryKey: ['catalog'], queryFn: api.catalog })
  const [query, setQuery] = useState('')
  const groups = useMemo(() => groupResources((cat.data?.resources ?? []).filter((r) => r.group !== 'pre' && r.kind !== 'dependency' && (f.versions === 'latest' || !r.dev_only)), query), [cat.data, query, f.versions])
  const toggle = (id: string) => set({ resourceIds: f.resourceIds.includes(id) ? f.resourceIds.filter((x) => x !== id) : [...f.resourceIds, id] })
  return (
    <div className="space-y-4">
      <fieldset className="space-y-3">
        <legend className="mb-2 text-sm text-muted-foreground">{t('install.steps.whichKoraResourcesShould')}</legend>
        <Radio name="res" checked={f.resourcesAll} onChange={() => set({ resourcesAll: true })} label={t('install.steps.everything')} hint={t('install.steps.allKoraResourcesThings')} />
        <Radio name="res" checked={!f.resourcesAll} onChange={() => set({ resourcesAll: false })} label={t('install.steps.letMePick')} hint={t('install.steps.coreWhatDependsAlways')} />
      </fieldset>
      {!f.resourcesAll && (
        <div className="space-y-3">
          <Field label={t('install.steps.search')}><Input type="search" value={query} onChange={(e) => setQuery(e.target.value)} /></Field>
          <div className="max-h-80 space-y-4 overflow-y-auto rounded-md border border-border p-3">
            {groups.map(([g, items]) => (
              <fieldset key={g} className="space-y-2">
                <legend className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{t(GROUP_LABEL[g])}</legend>
                {items.map((r) => <Checkbox key={r.id} label={r.title ?? r.id} hint={r.description} checked={f.resourceIds.includes(r.id)} onChange={() => toggle(r.id)} />)}
              </fieldset>
            ))}
          </div>
          <p className="text-sm text-muted-foreground">{t('install.steps.countPicked', { count: f.resourceIds.length })}</p>
        </div>
      )}
      <ErrorNotice error={cat.error} />
    </div>
  )
}

const DB_OPTIONS: { v: DbChoice; label: Msg; hint: Msg }[] = [
  { v: 'install_if_missing', label: msg('install.steps.setUpDatabaseMe'), hint: msg('install.steps.usesMariadbMysqlAlready') },
  { v: 'use_existing', label: msg('install.steps.useDatabaseAlreadyComputer'), hint: msg('install.steps.koraCreatesNewDatabase') },
  { v: 'skip', label: msg('install.steps.skipDatabase'), hint: msg('install.steps.connectDatabaseYourselfLater') },
]

export function DatabaseStep({ f, set }: StepProps) {
  const [result, setResult] = useState<{ ok: boolean; message: string; problem?: Problem | null } | null>(null)
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<unknown>(null)
  const problem = dbLoginProblem(f)
  useEffect(() => setResult(null), [f.dbCustom, f.dbUser, f.dbPass, f.dbHost, f.dbPort])
  const test = async () => {
    setBusy(true)
    setErr(null)
    try {
      setResult(await api.installs.checkDbLogin(f.dbCustom ? { host: f.dbHost, port: Number(f.dbPort), user: f.dbUser, password: f.dbPass } : null))
    } catch (e) {
      setErr(e)
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="space-y-4">
      <fieldset className="space-y-3">
        <legend className="mb-2 text-sm text-muted-foreground">{t('install.steps.whereShouldServerKeep')}</legend>
        {DB_OPTIONS.map((o) => <Radio key={o.v} name="db" checked={f.db === o.v} onChange={() => set({ db: o.v })} label={t(o.label)} hint={t(o.hint)} />)}
      </fieldset>
      {f.db !== 'skip' && (
        <fieldset className="space-y-3">
          <legend className="mb-2 text-sm font-semibold">{t('install.steps.databaseAdministratorLogin')}</legend>
          <Radio name="dbadmin" checked={!f.dbCustom} onChange={() => set({ dbCustom: false })} label={t('install.steps.automatic')} hint={t('install.steps.koraTriesNormalLocal')} />
          <Radio name="dbadmin" checked={f.dbCustom} onChange={() => set({ dbCustom: true })} label={t('install.steps.iTypeLogin')} hint={t('install.steps.usedOnceCreateNew')} />
          {f.dbCustom && (
            <div className="grid gap-3 sm:grid-cols-2">
              <Field label={t('install.steps.user')}><Input value={f.dbUser} onChange={(e) => set({ dbUser: e.target.value })} autoComplete="off" /></Field>
              <div />
              <Field label={t('install.steps.password')}><Input type="password" value={f.dbPass} onChange={(e) => set({ dbPass: e.target.value })} autoComplete="new-password" /></Field>
              <Field label={t('install.steps.typePasswordAgain')}><Input type="password" value={f.dbPass2} onChange={(e) => set({ dbPass2: e.target.value })} autoComplete="new-password" /></Field>
              <Field label={t('install.steps.host')}><Input value={f.dbHost} onChange={(e) => set({ dbHost: e.target.value })} /></Field>
              <Field label={t('install.steps.port')}><Input inputMode="numeric" value={f.dbPort} onChange={(e) => set({ dbPort: e.target.value })} /></Field>
              {problem && <p role="alert" className="text-sm text-danger-text sm:col-span-2">{problem}</p>}
            </div>
          )}
          <Button disabled={busy || !!problem} onClick={test}>{t('install.steps.testLogin')}</Button>
          {result && !result.ok && result.problem
            // The database is installed but stopped: the same "Start it for me" as everywhere, then the test runs again by itself.
            ? <ErrorNotice error={result.problem} onFixed={test} />
            : result && <Alert tone={result.ok ? 'success' : 'danger'}>{result.message}</Alert>}
          <ErrorNotice error={err} />
        </fieldset>
      )}
    </div>
  )
}

export function OptionsStep({ f, set, advanced }: StepProps) {
  return (
    <div className="space-y-4">
      <Checkbox label={t('install.steps.openGamePortFirewall')} hint={t('install.steps.letsPlayersOutsideComputer')} checked={f.firewall} onChange={(e) => set({ firewall: e.target.checked })} />
      {advanced && <Checkbox label={t('install.steps.alsoOpenTxadminPort')} hint={t('install.steps.doNeedUseTxadmin')} checked={f.txPublic} onChange={(e) => set({ txPublic: e.target.checked })} />}
      <Checkbox label={t('install.steps.installBackgroundService')} hint={t('install.steps.serverStartsComputerRestarts')} checked={f.service} onChange={(e) => set({ service: e.target.checked })} />
    </div>
  )
}

/** Where server licence keys are registered. */
export const CFX_KEYS_URL = 'https://portal.cfx.re/servers/registration-keys'

export function LicenceStep({ f, set }: StepProps) {
  const [show, setShow] = useState(false)
  const keys = useQuery({ queryKey: ['keys'], queryFn: api.keys.list, retry: false })
  const saved = keys.data?.keys ?? []
  const canManage = keys.data?.can_manage ?? true
  // The saved key the answers name is gone (renamed or deleted since): say so, and pick something else.
  const gone = f.licenceChoice === 'saved' && keys.isSuccess && !saved.some((k) => k.name === f.licenceName) ? f.licenceName : null
  const [goneName, setGoneName] = useState<string | null>(null)
  useEffect(() => {
    if (!gone) return
    setGoneName(gone)
    set(saved.length > 0 ? { licenceName: saved[0].name } : { licenceChoice: 'new', licenceName: '' })
  }, [gone]) // eslint-disable-line react-hooks/exhaustive-deps
  // Saving needs a readable keys file.
  useEffect(() => {
    if (keys.isError && f.saveKey) set({ saveKey: false })
  }, [keys.isError]) // eslint-disable-line react-hooks/exhaustive-deps
  const picking = f.licenceChoice === 'saved' && saved.length > 0
  const noKeysForYou = keys.isSuccess && saved.length === 0 && !canManage
  return (
    <div className="space-y-4">
      {goneName && <Alert tone="warning">{t('install.steps.savedLicenceKeyGonename', { goneName, value: canManage ? ' or add it again' : '' })}</Alert>}
      {keys.isError && <ErrorNotice error={keys.error} title={t('install.steps.koraCannotReadSaved')} />}
      {saved.length > 0 && (
        <fieldset className="space-y-2">
          <legend className="mb-1 text-sm text-muted-foreground">{t('install.steps.whichLicenceKeyShould')}</legend>
          {saved.map((k) => (
            <Radio key={k.name} name="licence-pick" checked={picking && f.licenceName === k.name} onChange={() => set({ licenceChoice: 'saved', licenceName: k.name })}
              label={k.name} hint={t('install.steps.keyEndingLastValue', { last4: k.last4, value: k.servers.length > 0 ? t('install.steps.usedJoin', { join: k.servers.join(', ') }) : '.' })} />
          ))}
          {canManage && <Radio name="licence-pick" checked={f.licenceChoice === 'new'} onChange={() => set({ licenceChoice: 'new' })} label={t('install.steps.addNewLicenceKey')} hint={t('install.steps.pasteKeyUsedHere')} />}
        </fieldset>
      )}
      {noKeysForYou && <Alert tone="info">{t('install.steps.thereNoSavedLicence')}</Alert>}
      {f.licenceChoice === 'new' && !noKeysForYou && (
        <>
          <p className="text-sm text-muted-foreground">{t('install.steps.everyFivemServerNeeds')}</p>
          <ol className="list-decimal space-y-1 pl-5 text-sm text-muted-foreground">
            <li>{t('install.steps.openCfxRePortal')}</li>
            <li>{t('install.steps.chooseServerKeysRegister')}</li>
            <li>{t('install.steps.copyKeyPasteBelow')}</li>
          </ol>
          <Button asChild><a href={CFX_KEYS_URL} target="_blank" rel="noreferrer">{rich('install.steps.openCfxRePortal2', { externalLink: <ExternalLink /> })}</a></Button>
          <div className="space-y-1.5">
            <Label htmlFor="licence-key">{t('install.steps.licenceKey')}</Label>
            <div className="flex gap-2">
              <Input id="licence-key" aria-describedby="licence-hint" type={show ? 'text' : 'password'} value={f.license} onChange={(e) => set({ license: e.target.value })} autoComplete="off" spellCheck={false} />
              <Button size="icon" aria-label={show ? t('install.steps.hideLicenceKey') : t('install.steps.showLicenceKey')} aria-pressed={show} onClick={() => setShow((x) => !x)}>{show ? <EyeOff /> : <Eye />}</Button>
            </div>
            <p id="licence-hint" className="text-xs text-muted-foreground">{t('install.steps.koraOnlyEverShows')}</p>
          </div>
          <Checkbox
            label={t('install.steps.saveKeyComputerNext')}
            hint={f.saveKey
              ? t('install.steps.keptPrivateFileKora')
              : t('install.steps.savedPasteAgainNext')}
            checked={f.saveKey} disabled={keys.isError} onChange={(e) => set({ saveKey: e.target.checked })}
          />
          {f.saveKey && (
            <Field label={t('install.steps.nameKey')} hint={t('install.steps.somethingRecogniseExampleMain')}>
              <Input value={f.saveKeyName} maxLength={40} onChange={(e) => set({ saveKeyName: e.target.value })} autoComplete="off" />
            </Field>
          )}
        </>
      )}
    </div>
  )
}

export function CfxStep({ f, set }: StepProps) {
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const [err, setErr] = useState<unknown>(null)
  const name = f.cfxUsername.trim()
  const edit = (v: string) => {
    set({ cfxUsername: v, cfxFound: null, cfxRemembered: false })
    setFailure(null)
    setErr(null)
  }
  const check = async () => {
    setBusy(true)
    setErr(null)
    setFailure(null)
    try {
      const r = await api.checks.cfx(name)
      if (r.ok && r.identifier) set({ cfxFound: { username: r.username ?? name, identifier: r.identifier }, cfxRemembered: false })
      else { set({ cfxFound: null }); setFailure(r.message) }
    } catch (e) {
      setErr(e)
    } finally {
      setBusy(false)
    }
  }
  const found = f.cfxFound
  return (
    <div className="space-y-4">
      <p className="text-sm font-semibold">{t('install.steps.signTxadminCfxRe')}</p>
      <p className="text-sm text-muted-foreground">{t('install.steps.txadminWebPageWhere')}</p>
      <div className="space-y-1.5">
        <Label htmlFor="cfx-username">{t('install.steps.cfxReUsername')}</Label>
        <div className="flex gap-2">
          <Input id="cfx-username" value={f.cfxUsername} onChange={(e) => edit(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter' && name && !busy) void check() }} autoComplete="off" spellCheck={false} />
          <Button disabled={busy || !name} onClick={check}>{t('install.steps.check')}</Button>
        </div>
      </div>
      <div aria-live="polite" className="space-y-2">
        {found && <Alert tone="success">{found.username ? t('install.steps.foundUsernameIdentifierAccount', { username: found.username, identifier: found.identifier }) : t('install.steps.keptFromEarlierAnswers', { identifier: found.identifier })}</Alert>}
        {found && f.cfxRemembered && <p className="text-xs text-muted-foreground">{t('install.steps.rememberedFromLastInstall')}</p>}
        {failure && <Alert tone="danger">{failure}</Alert>}
        {!found && !failure && !name && <p className="text-sm text-muted-foreground">{t('install.steps.withoutKoraCreatesLocal')}</p>}
      </div>
      <ErrorNotice error={err} />
      <Button disabled={!f.cfxUsername && !found} onClick={() => edit('')}>{t('install.steps.skipUseGeneratedLocal')}</Button>
    </div>
  )
}

export function GithubStep({ f, set, signedIn, onSignedIn }: StepProps & { signedIn: boolean; onSignedIn: () => void }) {
  const owners = useQuery({ queryKey: ['github-owners'], queryFn: api.github.owners, enabled: signedIn })
  const personal = (owners.data ?? []).find((o) => o.kind === 'user')
  const orgs = (owners.data ?? []).filter((o) => o.kind !== 'user')
  const usable = orgs.filter((o) => o.can_fork)
  const chosenOrg = usable.some((o) => o.login === f.githubOwner)
  // Two plain steps: personal account or organisation; an organisation then asks which one.
  const [where, setWhere] = useState<'personal' | 'org' | null>(null)
  const shown = where ?? (chosenOrg ? 'org' : f.githubOwner && personal?.login === f.githubOwner ? 'personal' : null)
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">{t('install.steps.eachResourceGetsOwn')}</p>
      <GithubSignIn onSignedIn={onSignedIn} />
      {signedIn && owners.isLoading && (
        <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">{rich('install.steps.loaderNodeLoadingGithub', { loader2: <Loader2 className="size-4 animate-spin" aria-hidden /> })}</p>
      )}
      {signedIn && !owners.isLoading && !owners.error && (
        <fieldset className="space-y-3">
          <legend className="sr-only">{t('install.steps.whereCopiesGo')}</legend>
          <Radio
            name="fork-where"
            checked={shown === 'personal'}
            disabled={!personal}
            onChange={() => { setWhere('personal'); if (personal) set({ githubOwner: personal.login, githubRemembered: false }) }}
            label={personal ? t('install.steps.myPersonalAccountLogin', { login: personal.login }) : t('install.steps.myPersonalAccount')}
            hint={f.githubRemembered && f.githubOwner && personal?.login === f.githubOwner ? t('install.steps.rememberedFromLastInstall2') : undefined}
          />
          <Radio
            name="fork-where"
            checked={shown === 'org'}
            disabled={usable.length === 0}
            onChange={() => { setWhere('org'); if (!chosenOrg) set({ githubOwner: '', githubRemembered: false }) }}
            label={t('install.steps.organisationIBelong')}
            hint={orgs.length === 0 ? t('install.steps.doBelongOrganisationKora') : usable.length === 0 ? t('install.steps.belongJoinButMay', { join: orgs.map((o) => o.login).join(', '), reason: orgs[0].reason ?? 'not allowed' }) : undefined}
          />
          {shown === 'org' && usable.length > 0 && (
            <Field label={t('install.steps.whichOrganisation')} hint={f.githubRemembered && chosenOrg ? t('install.steps.rememberedFromLastInstall2') : t('install.steps.ownCopiesCreatedThere')}>
              <Select value={chosenOrg ? f.githubOwner : ''} onChange={(e) => set({ githubOwner: e.target.value, githubRemembered: false })}>
                <option value="">{t('install.steps.choose')}</option>
                {orgs.map((o) => <option key={o.login} value={o.login} disabled={!o.can_fork}>{o.can_fork ? o.login : t('install.steps.loginMayCreateRepositories', { login: o.login })}</option>)}
              </Select>
            </Field>
          )}
        </fieldset>
      )}
      <ErrorNotice error={owners.error} />
    </div>
  )
}
