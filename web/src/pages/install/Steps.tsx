import { useEffect, useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Eye, EyeOff } from 'lucide-react'
import { api } from '@/api/client'
import type { DbChoice, PortsCheck } from '@/api/types'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox, Radio } from '@/components/ui/checkbox'
import { Input, Label, Select } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { GithubSignIn } from '@/components/GithubSignIn'
import { GROUP_LABEL, groupResources } from '@/lib/catalog'
import { slug, validatePorts } from '@/lib/utils'
import { dbLoginProblem, dirBase, folderFor, type WizardForm } from './form'

export interface StepProps {
  f: WizardForm
  set: (patch: Partial<WizardForm>) => void
  advanced: boolean
  defaultsDir: string
}

export function ModeStep({ f, set }: StepProps) {
  return (
    <fieldset className="space-y-3">
      <legend className="mb-2 text-sm text-muted-foreground">What kind of server is this?</legend>
      <Radio name="mode" checked={f.mode === 'production'} onChange={() => set({ mode: 'production' })} label="Production" hint="A server for players. Resources are installed from tested releases, and you choose when to update." />
      <Radio name="mode" checked={f.mode === 'development'} onChange={() => set({ mode: 'development' })} label="Development" hint="For building and changing resources. Kora forks them into your GitHub account, and the server can update itself on a schedule." />
    </fieldset>
  )
}

export function NameStep({ f, set, advanced, defaultsDir }: StepProps) {
  const base = dirBase(defaultsDir)
  return (
    <div className="space-y-4">
      <Field label="Server name" hint={`Used for the folder, the service and the database. Saved as "${slug(f.name) || '...'}".`}>
        <Input value={f.name} onChange={(e) => set({ name: e.target.value, ...(f.dirTouched ? {} : { dir: folderFor(base, e.target.value) }) })} autoComplete="off" />
      </Field>
      <Field label="Server folder" hint="Where the server files go. It is suggested from the name.">
        <Input value={f.dir} onChange={(e) => set({ dir: e.target.value, dirTouched: true })} autoComplete="off" />
      </Field>
      {advanced && (
        <Field label="Server name shown to players (hostname)" hint="Shown in the FiveM server list.">
          <Input value={f.hostname} onChange={(e) => set({ hostname: e.target.value })} />
        </Field>
      )}
    </div>
  )
}

export function PortsStep({ f, set, onOk }: StepProps & { onOk: (ok: boolean) => void }) {
  const [check, setCheck] = useState<PortsCheck | null>(null)
  const [err, setErr] = useState<unknown>(null)
  const problem = validatePorts(f.game, f.tx)
  useEffect(() => {
    if (problem) {
      setCheck(null)
      onOk(false)
      return
    }
    let stale = false
    onOk(false)
    const t = setTimeout(() => {
      api.installs.checkPorts({ game: Number(f.game), txadmin: Number(f.tx) }).then(
        (c) => { if (!stale) { setCheck(c); setErr(null); onOk(c.game.ok && c.txadmin.ok) } },
        (e) => { if (!stale) setErr(e) },
      )
    }, 250)
    return () => { stale = true; clearTimeout(t) }
  }, [f.game, f.tx, problem, onOk])
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
      <ErrorNotice error={err} />
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
              {(arts.data?.items ?? []).map((a) => <option key={a.build} value={a.build}>{a.build}{a.build === arts.data?.recommended ? ' (recommended)' : ''}{a.date ? `, ${a.date}` : ''}</option>)}
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
  const groups = useMemo(() => groupResources((cat.data?.resources ?? []).filter((r) => r.group !== 'pre' && r.kind !== 'dependency' && (f.mode === 'development' || !r.dev_only)), query), [cat.data, query, f.mode])
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
  { v: 'use_existing', label: 'Use the database already on this computer', hint: 'Kora creates a new database and user for this server. Your other databases are not touched.' },
  { v: 'install_if_missing', label: 'Install a database if there is none', hint: 'Installs MariaDB first when needed. This needs administrator rights.' },
  { v: 'skip', label: 'Skip the database', hint: 'You will connect a database yourself later.' },
]

export function DatabaseStep({ f, set }: StepProps) {
  const [result, setResult] = useState<{ ok: boolean; message: string } | null>(null)
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
          {result && <Alert tone={result.ok ? 'success' : 'danger'}>{result.message}</Alert>}
          <ErrorNotice error={err} />
        </fieldset>
      )}
    </div>
  )
}

export function OptionsStep({ f, set, advanced, appSupported }: StepProps & { appSupported: boolean }) {
  return (
    <div className="space-y-4">
      <Checkbox label="Open the game port in the firewall" hint="Lets players outside this computer connect. Needs administrator rights." checked={f.firewall} onChange={(e) => set({ firewall: e.target.checked })} />
      {advanced && <Checkbox label="Also open the txAdmin port" hint="Lets you reach txAdmin from other computers. Anyone who can reach it can try to sign in, so leave this off unless you need it." checked={f.txPublic} onChange={(e) => set({ txPublic: e.target.checked })} />}
      <Checkbox label="Install as a background service" hint="The server starts with the computer and restarts if it crashes. Needs administrator rights." checked={f.service} onChange={(e) => set({ service: e.target.checked })} />
      {appSupported && (
        <div className="space-y-2">
          <Checkbox label="Install the Kora app" hint="Adds Kora to the Start Menu so you can open the manager later." checked={f.app} onChange={(e) => set({ app: e.target.checked })} />
          {f.app && <div className="pl-7"><Checkbox label="Also put a shortcut on the desktop" checked={f.desktopShortcut} onChange={(e) => set({ desktopShortcut: e.target.checked })} /></div>}
        </div>
      )}
    </div>
  )
}

export function LicenceStep({ f, set }: StepProps) {
  const [show, setShow] = useState(false)
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">Every FiveM server needs its own licence key. You get one for free from the Cfx.re portal: sign in with your Cfx.re account, choose "Server keys", register a key for this server, and copy it here.</p>
      <div className="space-y-1.5">
        <Label htmlFor="licence-key">Licence key</Label>
        <div className="flex gap-2">
          <Input id="licence-key" aria-describedby="licence-hint" type={show ? 'text' : 'password'} value={f.license} onChange={(e) => set({ license: e.target.value })} autoComplete="off" spellCheck={false} />
          <Button size="icon" aria-label={show ? 'Hide the licence key' : 'Show the licence key'} aria-pressed={show} onClick={() => setShow((s) => !s)}>{show ? <EyeOff /> : <Eye />}</Button>
        </div>
        <p id="licence-hint" className="text-xs text-muted-foreground">It is only used to set up this server and is never shown again.</p>
      </div>
    </div>
  )
}

export function GithubStep({ f, set, signedIn, onSignedIn }: StepProps & { signedIn: boolean; onSignedIn: () => void }) {
  const owners = useQuery({ queryKey: ['github-owners'], queryFn: api.github.owners, enabled: signedIn })
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">A development server keeps its own copy (a fork) of each Kora resource on GitHub, so you can change them and share your changes.</p>
      <GithubSignIn onSignedIn={onSignedIn} />
      {signedIn && (
        <Field label="Put the forks in" hint="Your own account, or an organisation you belong to.">
          <Select value={f.githubOwner} onChange={(e) => set({ githubOwner: e.target.value })}>
            <option value="">Choose...</option>
            {(owners.data ?? []).map((o) => <option key={o.login} value={o.login}>{o.login} ({o.kind === 'user' ? 'your account' : 'organisation'})</option>)}
          </Select>
        </Field>
      )}
      <ErrorNotice error={owners.error} />
    </div>
  )
}
