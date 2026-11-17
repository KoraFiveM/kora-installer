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
      <legend className="mb-2 text-sm text-muted-foreground">Where should the resources come from?</legend>
      <Radio name="source" checked={f.source === 'clone'} onChange={() => set(sourcePatch(f, 'clone'))} label="Use Kora's resources as they are" hint="Plain copies (clones) of the original repositories. You do not need a GitHub account." />
      <Radio name="source" checked={f.source === 'fork'} onChange={() => set(sourcePatch(f, 'fork'))} label="Make my own copies on GitHub so I can change them" hint="Kora forks each resource into your GitHub account or an organisation you belong to. You can then change them, send pull requests and keep them up to date with the originals." />
    </fieldset>
  )
}

export function VersionsStep({ f, set }: StepProps) {
  return (
    <fieldset className="space-y-3">
      <legend className="mb-2 text-sm text-muted-foreground">Which versions should the server follow?</legend>
      <Radio name="versions" checked={f.versions === 'released'} onChange={() => set({ versions: 'released', versionsTouched: true })} label="Released versions (recommended, stable)" hint="Every resource is pinned to a tested release. You choose when to move to a newer one." />
      <Radio name="versions" checked={f.versions === 'latest'} onChange={() => set({ versions: 'latest', versionsTouched: true })} label="Latest changes (the main branch)" hint="Every resource follows the newest changes, including things that are not released yet. Good for building and testing." />
      <p className="text-xs text-muted-foreground">You can change this later in the server's settings, and put any single resource on another branch or pull request to try it.</p>
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
      <Field label="Server name" hint={`Used for the folder, the service and the database. Saved as "${slug(f.name) || '...'}".`}>
        <Input value={f.name} onChange={(e) => set({ name: e.target.value, ...(f.dirTouched ? {} : { dir: folderFor(base, e.target.value) }) })} autoComplete="off" />
      </Field>
      {showFolder ? (
        <Field label="Server folder" hint="Where the server files go. It is suggested from the name.">
          <Input value={f.dir} onChange={(e) => set({ dir: e.target.value, dirTouched: true })} autoComplete="off" />
        </Field>
      ) : (
        <p className="text-sm text-muted-foreground">
          The server files go in <code className="break-all font-mono text-xs text-foreground">{f.dir}</code>.{' '}
          <Button variant="link" className="h-auto p-0" onClick={() => setFolderOpen(true)}>Use another folder</Button>
        </p>
      )}
      {advanced && (
        <Field label="Server name shown to players (hostname)" hint="Shown in the FiveM server list.">
          <Input value={f.hostname} onChange={(e) => set({ hostname: e.target.value })} />
        </Field>
      )}
      {quick && (showKind ? (
        <div className="space-y-4">
          <SourceStep f={f} set={set} advanced={advanced} defaultsDir={defaultsDir} />
          <VersionsStep f={f} set={set} advanced={advanced} defaultsDir={defaultsDir} />
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">
          The resources are plain copies of the originals, at released versions.{' '}
          <Button variant="link" className="h-auto p-0" onClick={() => setKindOpen(true)}>Choose where they come from</Button>
        </p>
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
            <span className={l.ok ? 'text-success' : 'text-destructive'}>{l.text}</span>
            {l.suggestion && <Button size="sm" onClick={() => set({ [field]: String(l.suggestion) } as Partial<WizardForm>)}>Use {l.suggestion}</Button>}
          </div>
        )}
      </div>
    )
  }
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">Players connect to the game port. txAdmin, the web page where you manage the server, uses the second port. Kora checks they are free.</p>
      <div className="grid gap-4 sm:grid-cols-2">
        {line('Game port', 'game', 'game')}
        {line('txAdmin port', 'txadmin', 'tx')}
      </div>
      {problem && <p role="alert" className="text-sm text-destructive">{problem}</p>}
      <ErrorNotice error={ports.error} />
      {ports.state === 'unknown' && <Button onClick={ports.retry}>Check the ports again</Button>}
    </div>
  )
}

export function BuildStep({ f, set, advanced }: StepProps) {
  const arts = useQuery({ queryKey: ['artifacts'], queryFn: api.artifacts })
  return (
    <div className="space-y-4">
      <fieldset className="space-y-3">
        <legend className="mb-2 text-sm text-muted-foreground">Which version of the FiveM server should be installed?</legend>
        <Radio name="art" checked={f.artifact === 'recommended'} onChange={() => set({ artifact: 'recommended' })} label="The recommended build" hint={arts.data?.recommended ? `Build ${arts.data.recommended}. The one Cfx.re suggests.` : 'The one Cfx.re suggests.'} />
        <Radio name="art" checked={f.artifact === 'pick'} onChange={() => set({ artifact: 'pick' })} label="Pick a build from the list" />
        {f.artifact === 'pick' && (
          <Field label="Build">
            <Select value={f.artifactBuild ?? ''} onChange={(e) => set({ artifactBuild: e.target.value ? Number(e.target.value) : null })}>
              <option value="">Choose a build</option>
              {(arts.data?.items ?? []).map((a) => <option key={a.build} value={a.build}>{a.build}{a.build === arts.data?.recommended ? ' (recommended)' : ''}</option>)}
            </Select>
          </Field>
        )}
        <Radio name="art" checked={f.artifact === 'custom'} onChange={() => set({ artifact: 'custom' })} label="Use my own download address" />
        {f.artifact === 'custom' && <Field label="Download address" hint="The link to a server.zip (Windows) or fx.tar.xz (Linux) from Cfx.re."><Input value={f.artifactUrl} onChange={(e) => set({ artifactUrl: e.target.value })} placeholder="https://..." /></Field>}
      </fieldset>
      <ErrorNotice error={arts.error} />
      {advanced && <Field label="Game build (optional)" hint="Forces a GTA V game build, for example 3095. Leave empty to use the default."><Input inputMode="numeric" value={f.gameBuild} onChange={(e) => set({ gameBuild: e.target.value })} /></Field>}
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
        <legend className="mb-2 text-sm text-muted-foreground">Which Kora resources should be installed?</legend>
        <Radio name="res" checked={f.resourcesAll} onChange={() => set({ resourcesAll: true })} label="Everything" hint="All Kora resources and the things they need. The easiest choice." />
        <Radio name="res" checked={!f.resourcesAll} onChange={() => set({ resourcesAll: false })} label="Let me pick" hint="The core and what it depends on are always installed. You choose the extras." />
      </fieldset>
      {!f.resourcesAll && (
        <div className="space-y-3">
          <Field label="Search"><Input type="search" value={query} onChange={(e) => setQuery(e.target.value)} /></Field>
          <div className="max-h-80 space-y-4 overflow-y-auto rounded-md border border-border p-3">
            {groups.map(([g, items]) => (
              <fieldset key={g} className="space-y-2">
                <legend className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{GROUP_LABEL[g]}</legend>
                {items.map((r) => <Checkbox key={r.id} label={r.title ?? r.id} hint={r.description} checked={f.resourceIds.includes(r.id)} onChange={() => toggle(r.id)} />)}
              </fieldset>
            ))}
          </div>
          <p className="text-sm text-muted-foreground">{f.resourceIds.length} picked.</p>
        </div>
      )}
      <ErrorNotice error={cat.error} />
    </div>
  )
}

const DB_OPTIONS: { v: DbChoice; label: string; hint: string }[] = [
  { v: 'install_if_missing', label: 'Set up the database for me (recommended)', hint: 'Uses MariaDB or MySQL if it is already here, installs MariaDB if not (installing it needs administrator rights). Your other databases are not touched.' },
  { v: 'use_existing', label: 'Use the database already on this computer', hint: 'Kora creates a new database and user for this server. Your other databases are not touched. The install stops if no database is found.' },
  { v: 'skip', label: 'Skip the database', hint: 'You will connect a database yourself later.' },
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
        <legend className="mb-2 text-sm text-muted-foreground">Where should the server keep its data?</legend>
        {DB_OPTIONS.map((o) => <Radio key={o.v} name="db" checked={f.db === o.v} onChange={() => set({ db: o.v })} label={o.label} hint={o.hint} />)}
      </fieldset>
      {f.db !== 'skip' && (
        <fieldset className="space-y-3">
          <legend className="mb-2 text-sm font-semibold">Database administrator login</legend>
          <Radio name="dbadmin" checked={!f.dbCustom} onChange={() => set({ dbCustom: false })} label="Automatic" hint="Kora tries the normal local login. Works on most fresh installs." />
          <Radio name="dbadmin" checked={f.dbCustom} onChange={() => set({ dbCustom: true })} label="I will type the login" hint="Used once to create the new database and user, and not kept." />
          {f.dbCustom && (
            <div className="grid gap-3 sm:grid-cols-2">
              <Field label="User"><Input value={f.dbUser} onChange={(e) => set({ dbUser: e.target.value })} autoComplete="off" /></Field>
              <div />
              <Field label="Password"><Input type="password" value={f.dbPass} onChange={(e) => set({ dbPass: e.target.value })} autoComplete="new-password" /></Field>
              <Field label="Type the password again"><Input type="password" value={f.dbPass2} onChange={(e) => set({ dbPass2: e.target.value })} autoComplete="new-password" /></Field>
              <Field label="Host"><Input value={f.dbHost} onChange={(e) => set({ dbHost: e.target.value })} /></Field>
              <Field label="Port"><Input inputMode="numeric" value={f.dbPort} onChange={(e) => set({ dbPort: e.target.value })} /></Field>
              {problem && <p role="alert" className="text-sm text-destructive sm:col-span-2">{problem}</p>}
            </div>
          )}
          <Button disabled={busy || !!problem} onClick={test}>Test login</Button>
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
      <Checkbox label="Open the game port in the firewall" hint="Lets players outside this computer connect. Needs administrator rights." checked={f.firewall} onChange={(e) => set({ firewall: e.target.checked })} />
      {advanced && <Checkbox label="Also open the txAdmin port" hint="Lets you reach txAdmin from other computers. Anyone who can reach it can try to sign in, so leave this off unless you need it." checked={f.txPublic} onChange={(e) => set({ txPublic: e.target.checked })} />}
      <Checkbox label="Install as a background service" hint="The server starts with the computer and restarts if it crashes. Needs administrator rights." checked={f.service} onChange={(e) => set({ service: e.target.checked })} />
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
      {goneName && <Alert tone="warning">The saved licence key &quot;{goneName}&quot; is not there any more (it was renamed or deleted). Pick another one{canManage ? ' or add it again' : ''}.</Alert>}
      {keys.isError && <ErrorNotice error={keys.error} title="Kora cannot read your saved licence keys" />}
      {saved.length > 0 && (
        <fieldset className="space-y-2">
          <legend className="mb-1 text-sm text-muted-foreground">Which licence key should this server use?</legend>
          {saved.map((k) => (
            <Radio key={k.name} name="licence-pick" checked={picking && f.licenceName === k.name} onChange={() => set({ licenceChoice: 'saved', licenceName: k.name })}
              label={k.name} hint={`Key ending ${k.last4}${k.servers.length > 0 ? `. Used by ${k.servers.join(', ')}.` : '.'}`} />
          ))}
          {canManage && <Radio name="licence-pick" checked={f.licenceChoice === 'new'} onChange={() => set({ licenceChoice: 'new' })} label="Add a new licence key" hint="Paste a key you have not used here before." />}
        </fieldset>
      )}
      {noKeysForYou && <Alert tone="info">There are no saved licence keys yet. Only an administrator can add one: ask them to add it in Settings, under Licence keys.</Alert>}
      {f.licenceChoice === 'new' && !noKeysForYou && (
        <>
          <p className="text-sm text-muted-foreground">Every FiveM server needs its own licence key. It is free. To get one:</p>
          <ol className="list-decimal space-y-1 pl-5 text-sm text-muted-foreground">
            <li>Open the Cfx.re portal and sign in.</li>
            <li>Choose &quot;Server keys&quot;, then register a new key.</li>
            <li>Copy the key and paste it below.</li>
          </ol>
          <Button asChild><a href={CFX_KEYS_URL} target="_blank" rel="noreferrer">Open the Cfx.re portal <ExternalLink /></a></Button>
          <div className="space-y-1.5">
            <Label htmlFor="licence-key">Licence key</Label>
            <div className="flex gap-2">
              <Input id="licence-key" aria-describedby="licence-hint" type={show ? 'text' : 'password'} value={f.license} onChange={(e) => set({ license: e.target.value })} autoComplete="off" spellCheck={false} />
              <Button size="icon" aria-label={show ? 'Hide the licence key' : 'Show the licence key'} aria-pressed={show} onClick={() => setShow((x) => !x)}>{show ? <EyeOff /> : <Eye />}</Button>
            </div>
            <p id="licence-hint" className="text-xs text-muted-foreground">Kora only ever shows the last 4 characters of a key.</p>
          </div>
          <Checkbox
            label="Save this key on this computer for next time"
            hint={f.saveKey
              ? 'Kept in a private file in the Kora folder that only you can read. Next time you pick it by name instead of pasting it, and an install that stops can carry on even if Kora Panel is restarted.'
              : 'Not saved. You will paste it again next time, and an install that stops can only carry on while Kora Panel keeps running.'}
            checked={f.saveKey} disabled={keys.isError} onChange={(e) => set({ saveKey: e.target.checked })}
          />
          {f.saveKey && (
            <Field label="Name for this key" hint='Something you will recognise, for example "Main" or "Test server".'>
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
      <p className="text-sm font-semibold">Sign in to txAdmin with your Cfx.re account</p>
      <p className="text-sm text-muted-foreground">txAdmin is the web page where you manage the server. Give your Cfx.re username and you can sign in there with your Cfx.re account. Only the username is sent, to forum.cfx.re, to look up which account it belongs to.</p>
      <div className="space-y-1.5">
        <Label htmlFor="cfx-username">Cfx.re username</Label>
        <div className="flex gap-2">
          <Input id="cfx-username" value={f.cfxUsername} onChange={(e) => edit(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter' && name && !busy) void check() }} autoComplete="off" spellCheck={false} />
          <Button disabled={busy || !name} onClick={check}>Check</Button>
        </div>
      </div>
      <div aria-live="polite" className="space-y-2">
        {found && <Alert tone="success">{found.username ? `Found ${found.username} (${found.identifier}). This account will be the txAdmin admin.` : `Kept from your earlier answers: ${found.identifier}. This account will be the txAdmin admin.`}</Alert>}
        {found && f.cfxRemembered && <p className="text-xs text-muted-foreground">Remembered from your last install. Type another username and press Check to change it, or use Skip below.</p>}
        {failure && <Alert tone="danger">{failure}</Alert>}
        {!found && !failure && !name && <p className="text-sm text-muted-foreground">Without it, Kora creates a local admin with a generated password.</p>}
      </div>
      <ErrorNotice error={err} />
      <Button disabled={!f.cfxUsername && !found} onClick={() => edit('')}>Skip: use a generated local admin instead (password saved in the server folder)</Button>
    </div>
  )
}

export function GithubStep({ f, set, signedIn, onSignedIn }: StepProps & { signedIn: boolean; onSignedIn: () => void }) {
  const owners = useQuery({ queryKey: ['github-owners'], queryFn: api.github.owners, enabled: signedIn })
  const personal = (owners.data ?? []).find((o) => o.kind === 'user')
  const orgs = (owners.data ?? []).filter((o) => o.kind !== 'user')
  const chosenOrg = orgs.some((o) => o.login === f.githubOwner)
  // Two plain steps: personal account or organisation; an organisation then asks which one.
  const [where, setWhere] = useState<'personal' | 'org' | null>(null)
  const shown = where ?? (chosenOrg ? 'org' : f.githubOwner && personal?.login === f.githubOwner ? 'personal' : null)
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">Each resource gets your own copy (a fork) on GitHub, so you can change it and share your changes. Where should the copies go?</p>
      <GithubSignIn onSignedIn={onSignedIn} />
      {signedIn && owners.isLoading && (
        <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" aria-hidden /> Loading your GitHub accounts and organisations. This can take a few seconds.
        </p>
      )}
      {signedIn && !owners.isLoading && !owners.error && (
        <fieldset className="space-y-3">
          <legend className="sr-only">Where the copies go</legend>
          <Radio
            name="fork-where"
            checked={shown === 'personal'}
            disabled={!personal}
            onChange={() => { setWhere('personal'); if (personal) set({ githubOwner: personal.login, githubRemembered: false }) }}
            label={personal ? `My personal account (${personal.login})` : 'My personal account'}
            hint={f.githubRemembered && f.githubOwner && personal?.login === f.githubOwner ? 'Remembered from your last install: pick another to change it.' : undefined}
          />
          <Radio
            name="fork-where"
            checked={shown === 'org'}
            disabled={orgs.length === 0}
            onChange={() => { setWhere('org'); if (!chosenOrg) set({ githubOwner: '', githubRemembered: false }) }}
            label="An organisation I belong to"
            hint={orgs.length === 0 ? 'You do not belong to an organisation that Kora can put copies in. Ask an owner of the organisation to let you create repositories, then sign in to GitHub again.' : undefined}
          />
          {shown === 'org' && orgs.length > 0 && (
            <Field label="Which organisation?" hint={f.githubRemembered && chosenOrg ? 'Remembered from your last install: pick another to change it.' : 'Your own copies are created there, so you need the right to create repositories in it.'}>
              <Select value={chosenOrg ? f.githubOwner : ''} onChange={(e) => set({ githubOwner: e.target.value, githubRemembered: false })}>
                <option value="">Choose...</option>
                {orgs.map((o) => <option key={o.login} value={o.login}>{o.login}</option>)}
              </Select>
            </Field>
          )}
        </fieldset>
      )}
      <ErrorNotice error={owners.error} />
    </div>
  )
}
