// The steps of "Bring a server from a backup or another machine". Kept in their own module (the install wizard's
// steps are another piece of work): file → password → what will happen → progress → done.
import { useRef, useState } from 'react'
import { useMutation } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { FileArchive, Upload } from 'lucide-react'
import type { ArchivedServer, BackupEvent, Clash, DbAdmin, ForkChoice, ImportChoices, ImportPreview, ImportResult, ImportSource, JobError, RunMode } from '@/api/types'
import { api } from '@/api/client'
import { StyleChip } from '@/components/Status'
import { browseForFile, inShell } from '@/lib/shell'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox, Radio } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Progress } from '@/components/ui/progress'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'

export const FILE_HINT = 'A file made on the old computer with "Move this server" (it ends in .tar.gz.age), or any Kora server backup (.tar.gz or .tar.gz.age).'

export function FileStep({ local, busy, error, archived, onFile, onPath, onArchived }: { local: boolean; busy: boolean; error: unknown; archived: ArchivedServer[]; onFile: (f: File) => void; onPath: (p: string) => void; onArchived: (s: ArchivedServer, id: string) => void }) {
  const input = useRef<HTMLInputElement>(null)
  const [path, setPath] = useState('')
  const [drag, setDrag] = useState(false)
  const pick = (files: FileList | null) => {
    const f = files?.[0]
    if (f) onFile(f)
  }
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">{FILE_HINT}</p>
      <div
        role="group"
        aria-label="Drop the backup file here"
        className={`flex flex-col items-center gap-3 rounded-md border-2 border-dashed p-6 text-center text-sm ${drag ? 'border-primary bg-primary/5' : 'border-border'}`}
        onDragOver={(e) => { e.preventDefault(); setDrag(true) }}
        onDragLeave={() => setDrag(false)}
        onDrop={(e) => { e.preventDefault(); setDrag(false); pick(e.dataTransfer.files) }}
      >
        <Upload className="size-6 text-muted-foreground" />
        <p>Drop the file here, or choose it. It is sent to the computer that runs Kora and stays there.</p>
        <input ref={input} id="import-file" type="file" className="sr-only" disabled={busy} onChange={(e) => { pick(e.target.files); e.target.value = '' }} />
        <Button asChild variant="secondary" size="sm"><label htmlFor="import-file" className={busy ? 'pointer-events-none opacity-50' : 'cursor-pointer'}>Choose the backup file</label></Button>
        {busy && <p className="text-muted-foreground" aria-live="polite">Sending the file… a big file takes a while. Keep this page open.</p>}
      </div>
      {local && (
        <form className="space-y-2" onSubmit={(e) => { e.preventDefault(); if (path.trim()) onPath(path.trim()) }}>
          <Field label="Or give the file's path on this computer" hint="For example /mnt/usb/old-server.tar.gz.age or D:\Moves\old-server.tar.gz.age. The file is read where it is."><Input value={path} placeholder="/path/to/file.tar.gz.age" onChange={(e) => setPath(e.target.value)} /></Field>
          <div className="flex flex-wrap gap-2">
            <Button type="submit" disabled={!path.trim() || busy}>Check this file</Button>
            {inShell() && <Button type="button" disabled={busy} onClick={async () => { const f = await browseForFile('Choose the backup file', ['age', 'gz']); if (f) { setPath(f); onPath(f) } }}>Browse...</Button>}
          </div>
        </form>
      )}
      {archived.length > 0 && (
        <div role="group" className="space-y-2" aria-label="Backups of servers that no longer exist">
          <p className="text-sm font-semibold">Backups of servers that no longer exist</p>
          <p className="text-xs text-muted-foreground">Kora keeps the backups of a server you deleted. Restore one as a new server:</p>
          <ul className="divide-y divide-border rounded-md border border-border">
            {archived.flatMap((a) => a.backups.map((b) => (
              <li key={`${a.name}-${a.deleted_unix}-${b.id}`} className="flex flex-wrap items-center gap-2 p-2 text-sm">
                <span className="font-medium">{a.name}</span>
                <span className="text-xs text-muted-foreground">backup of {new Date(b.created_unix * 1000).toLocaleString()}{b.encrypted ? ' · password protected' : ''}</span>
                <Button size="sm" className="ml-auto" disabled={busy} onClick={() => onArchived(a, b.id)}>Restore as a new server</Button>
              </li>
            )))}
          </ul>
        </div>
      )}
      <ErrorNotice error={error} />
    </div>
  )
}

export function PasswordStep({ name, error, busy, onPassword }: { name: string; error: unknown; busy: boolean; onPassword: (p: string) => void }) {
  const [pw, setPw] = useState('')
  return (
    <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); if (pw) onPassword(pw) }}>
      <p className="text-sm"><FileArchive className="mr-1 inline size-4" /> <span className="font-mono text-xs">{name}</span> is protected with a password.</p>
      <Field label="Its password"><Input type="password" autoComplete="off" value={pw} onChange={(e) => setPw(e.target.value)} /></Field>
      <p className="text-xs text-muted-foreground">The password was chosen on the old computer when the file was made. Kora does not keep passwords and cannot recover a lost one.</p>
      <Button type="submit" variant="default" disabled={!pw || busy}>Open the file</Button>
      <ErrorNotice error={error} />
    </form>
  )
}

const KIND_FIELD: Record<string, keyof ImportChoices> = { name: 'name', game_port: 'game_port', txadmin_port: 'txadmin_port' }

function ClashRow({ c, onUse }: { c: Clash; onUse: (c: Clash) => void }) {
  return (
    <li className="flex flex-wrap items-center gap-2 text-sm">
      <span>{c.message}</span>
      {c.suggestion && KIND_FIELD[c.kind] && <Button size="sm" onClick={() => onUse(c)}>Use {c.suggestion}</Button>}
      {c.kind === 'folder' && <span className="text-muted-foreground">Choose another folder below.</span>}
    </li>
  )
}

export function PreviewStep({ preview, choices, setChoices, key_, setKey, forkChoice, setForkChoice, admin, setAdmin, start, setStart, runMode, setRunMode, busy, onGo }: {
  preview: ImportPreview
  choices: ImportChoices
  setChoices: (c: ImportChoices) => void
  key_: string | null
  setKey: (k: string | null) => void
  forkChoice: ForkChoice
  setForkChoice: (c: ForkChoice) => void
  admin: DbAdmin | null
  setAdmin: (a: DbAdmin | null) => void
  start: boolean
  setStart: (b: boolean) => void
  /** How the new server runs (`null`: not asked, because this computer has no container engine and the backup was no container). */
  runMode: RunMode | null
  setRunMode: (m: RunMode) => void
  busy: boolean
  onGo: () => void
}) {
  const s = preview.server
  const portBad = choices.game_port < 1024 || choices.txadmin_port < 1024 || choices.game_port === choices.txadmin_port
  const useClash = (c: Clash) => {
    const f = KIND_FIELD[c.kind]
    if (!f || !c.suggestion) return
    setChoices({ ...choices, [f]: f === 'name' ? c.suggestion : Number(c.suggestion) })
  }
  return (
    <div className="space-y-5">
      {s && (
        <div className="space-y-1 text-sm">
          <p><span className="font-semibold">{s.name}</span> <Badge tone="plain">made on {s.os === 'windows' ? 'Windows' : 'Linux'}</Badge> <StyleChip source={s.source} versions={s.versions} /></p>
          <p className="text-muted-foreground">{preview.by_reference} resources will be fetched again, {preview.in_full} come from the file{preview.database ? ` · database: ${preview.database.kind === 'none' ? 'none' : 'included'}` : ''}.</p>
        </div>
      )}
      {preview.clashes.length > 0 && (
        <Alert tone="warning" title="Something is already using these on this computer">
          <ul className="space-y-2">{preview.clashes.map((c) => <ClashRow key={c.kind} c={c} onUse={useClash} />)}</ul>
        </Alert>
      )}
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Name of the server here"><Input value={choices.name} onChange={(e) => setChoices({ ...choices, name: e.target.value })} /></Field>
        <Field label="Folder for it"><Input value={choices.dir} onChange={(e) => setChoices({ ...choices, dir: e.target.value })} /></Field>
        <Field label="Game port"><Input type="number" value={choices.game_port} onChange={(e) => setChoices({ ...choices, game_port: Number(e.target.value) })} /></Field>
        <Field label="txAdmin port" error={portBad ? 'Use two different ports of 1024 or more.' : null}><Input type="number" value={choices.txadmin_port} onChange={(e) => setChoices({ ...choices, txadmin_port: Number(e.target.value) })} /></Field>
      </div>
      <div className="space-y-2" aria-label="What will be created">
        <p className="text-sm font-semibold">Kora will create:</p>
        <ul className="list-disc space-y-1 pl-5 text-sm">{preview.will_create.map((l, i) => <li key={i}>{l}</li>)}</ul>
      </div>
      {preview.fork && !preview.fork.usable && (
        <fieldset className="space-y-2">
          <legend className="mb-1 text-sm font-semibold">Your copies on GitHub ({preview.fork.owner})</legend>
          <Alert tone="warning">{preview.fork.message}</Alert>
          <Radio name="fork" checked={forkChoice === 'keep'} onChange={() => setForkChoice('keep')} label="Keep the remotes as recorded" hint="Resources are fetched from the old owner's copies. Private ones may not be reachable from here." />
          <Radio name="fork" checked={forkChoice === 'own_copies'} onChange={() => setForkChoice('own_copies')} disabled={!preview.fork.signed_in_as} label="Make my own copies here" hint={preview.fork.signed_in_as ? `Kora forks the original repositories into ${preview.fork.signed_in_as} on GitHub and the server follows those.` : 'Sign in to GitHub on this computer first (the System check page).'} />
          <Radio name="fork" checked={forkChoice === 'originals'} onChange={() => setForkChoice('originals')} label="Use the originals" hint="Plain copies of the original repositories, no GitHub account needed. The server becomes a server of plain copies here." />
        </fieldset>
      )}
      {(runMode !== null || preview.run_mode === 'container') && (
        <fieldset className="space-y-2">
          <legend className="mb-1 text-sm font-semibold">How the server runs here</legend>
          {preview.run_mode === 'container' && <p className="text-sm text-muted-foreground">The server you are bringing in ran in a container.</p>}
          <Radio name="runmode" checked={runMode !== 'container'} onChange={() => setRunMode('service')} label="As a system service" hint="Starts with the computer. Setting the service up needs administrator rights (Kora shows what to do afterwards)." />
          <Radio name="runmode" checked={runMode === 'container'} onChange={() => setRunMode('container')} disabled={!preview.container_engine} label="In a container" hint={preview.container_engine ? `Uses ${preview.container_engine}. The database goes into a MariaDB container next to the server.` : 'This computer has no container engine (Podman or Docker) that Kora can use.'} />
        </fieldset>
      )}
      {preview.db_login && !preview.db_login.ok && runMode !== 'container' && <DbLoginForm note={preview.db_login.message} setAdmin={setAdmin} />}
      <fieldset className="space-y-2">
        <legend className="mb-1 text-sm font-semibold">Licence key</legend>
        <Alert tone="warning">{preview.licence_note}</Alert>
        <Radio name="key" checked={key_ === null} onChange={() => setKey(null)} label="Use the licence key that is in the file" />
        {preview.saved_keys.length > 0 && (
          <Radio name="key" checked={key_ !== null} onChange={() => setKey(preview.saved_keys[0])} label="Use one of my saved keys instead" hint={key_ !== null ? (
            <select aria-label="Saved licence key" className="mt-1 h-9 rounded-md border border-input bg-ink px-3 text-sm" value={key_} onChange={(e) => setKey(e.target.value)}>{preview.saved_keys.map((k) => <option key={k}>{k}</option>)}</select>
          ) : undefined} />
        )}
      </fieldset>
      <div className="space-y-2">
        <p className="text-sm font-semibold">What does not come along:</p>
        <ul className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">{preview.wont_carry.map((l, i) => <li key={i}>{l}</li>)}</ul>
      </div>
      {preview.warnings.map((w, i) => <Alert key={i} tone="info">{w}</Alert>)}
      <Checkbox checked={start} onChange={(e) => setStart(e.target.checked)} label="Start the server when it is ready" />
      <Button variant="default" disabled={busy || portBad || !choices.name.trim() || !choices.dir.trim() || (preview.db_login?.ok === false && !admin && runMode !== 'container')} onClick={onGo}>Bring the server in</Button>
    </div>
  )
}

/** The database administrator login, when the automatic one does not work: the same choice the install wizard's database step offers. Used once, never stored. */
function DbLoginForm({ note, setAdmin }: { note: string; setAdmin: (a: DbAdmin | null) => void }) {
  const [f, setF] = useState({ host: '127.0.0.1', port: 3306, user: 'root', password: '' })
  const test = useMutation({ mutationFn: () => api.installs.checkDbLogin({ host: f.host, port: f.port, user: f.user, password: f.password }) })
  return (
    <fieldset className="space-y-3">
      <legend className="mb-1 text-sm font-semibold">Database administrator login</legend>
      <Alert tone="warning">{note} Kora needs an administrator login of this computer's database server once, to create the new database. It is used for this import only and never saved.</Alert>
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Database server address"><Input value={f.host} onChange={(e) => { setF({ ...f, host: e.target.value }); setAdmin(null) }} /></Field>
        <Field label="Port"><Input type="number" value={f.port} onChange={(e) => { setF({ ...f, port: Number(e.target.value) }); setAdmin(null) }} /></Field>
        <Field label="Administrator user"><Input value={f.user} onChange={(e) => { setF({ ...f, user: e.target.value }); setAdmin(null) }} /></Field>
        <Field label="Administrator password"><Input type="password" autoComplete="off" value={f.password} onChange={(e) => { setF({ ...f, password: e.target.value }); setAdmin(null) }} /></Field>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" disabled={test.isPending || !f.user} onClick={() => test.mutate(undefined, { onSuccess: (r) => setAdmin(r.ok ? { host: f.host, port: f.port, user: f.user, password: f.password } : null) })}>Test this login</Button>
        {test.data && <span className={`text-sm ${test.data.ok ? 'text-success' : 'text-danger-text'}`} role="status">{test.data.message}</span>}
      </div>
      <ErrorNotice error={test.error} />
    </fieldset>
  )
}

export function RunStep({ events, failed, onRetry, onStartOver }: { events: BackupEvent[]; failed: JobError | null; onRetry: () => void; onStartOver: () => void }) {
  const last = events.at(-1)
  return (
    <div className="space-y-4" aria-live="polite">
      {!failed && (
        <>
          <Progress value={last?.percent ?? 0} label="Import progress" />
          <p className="text-sm text-muted-foreground">{last?.message ?? 'Starting…'}</p>
        </>
      )}
      {failed && (
        <>
          <ErrorNotice error={failed} onRetry={onRetry} />
          <div className="flex flex-wrap gap-2">
            <Button variant="default" onClick={onRetry}>Carry on from where it stopped</Button>
            <Button variant="outline" onClick={onStartOver}>Start over (removes what was made)</Button>
          </div>
        </>
      )}
    </div>
  )
}

export function DoneStep({ r }: { r: ImportResult }) {
  return (
    <div className="space-y-4">
      <Alert tone={r.problems.length ? 'warning' : 'success'} title={`${r.name} is imported`}>
        <p>It is in <span className="font-mono text-xs">{r.dir}</span>{r.started ? ' and it was started' : ''}.</p>
        {r.resources_fetched.length > 0 && <ul className="mt-1 list-disc pl-5">{r.resources_fetched.map((m, i) => <li key={i}>{m}</li>)}</ul>}
        {r.notes.map((n, i) => <p key={i} className="mt-1 text-muted-foreground">{n}</p>)}
      </Alert>
      {r.problems.length > 0 && <Alert tone="danger" title="What did not work"><ul className="list-disc pl-5">{r.problems.map((m, i) => <li key={i}>{m}</li>)}</ul></Alert>}
      <div className="space-y-1">
        <p className="text-sm font-semibold">Next, on this computer</p>
        <ul className="list-disc space-y-1 pl-5 text-sm">{r.next_steps.map((m, i) => <li key={i}>{m}</li>)}</ul>
      </div>
      <div className="space-y-1">
        <p className="text-sm font-semibold">Remove it from the old computer</p>
        <p className="text-sm text-muted-foreground">Kora does not reach into the old computer. When you are sure this one works:</p>
        <ol className="list-decimal space-y-1 pl-5 text-sm">{r.old_machine_steps.map((m, i) => <li key={i}>{m}</li>)}</ol>
      </div>
      <Button asChild variant="default"><Link to={`/servers/${encodeURIComponent(r.name)}`}>Open {r.name}</Link></Button>
    </div>
  )
}

export type { ImportSource }
