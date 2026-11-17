// The steps of "Bring a server from a backup or another machine". Kept in their own module (the install wizard's
// steps are another piece of work): file → password → what will happen → progress → done.
import { useRef, useState } from 'react'
import { useMutation } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { FileArchive, Upload } from 'lucide-react'
import type { ArchivedServer, BackupEvent, Clash, DbAdmin, ForkChoice, ImportChoices, ImportPreview, ImportResult, ImportSource, JobError } from '@/api/types'
import { api } from '@/api/client'
import { StyleChip } from '@/components/Status'
import { browseForFile, inShell } from '@/lib/shell'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox, Radio } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Announcer, progressSentence } from '@/components/Announcer'
import { Progress } from '@/components/ui/progress'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { t, msg } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/backups'

export const FILE_HINT = msg('backups.steps.fileMadeOldComputer')

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
      <p className="text-sm text-muted-foreground">{t(FILE_HINT)}</p>
      <div
        role="group"
        aria-label={t('backups.steps.dropBackupFileHere')}
        className={`flex flex-col items-center gap-3 rounded-md border-2 border-dashed p-6 text-center text-sm ${drag ? 'border-primary bg-primary/5' : 'border-border'}`}
        onDragOver={(e) => { e.preventDefault(); setDrag(true) }}
        onDragLeave={() => setDrag(false)}
        onDrop={(e) => { e.preventDefault(); setDrag(false); pick(e.dataTransfer.files) }}
      >
        <Upload className="size-6 text-muted-foreground" />
        <p>{t('backups.steps.dropFileHereChoose')}</p>
        <input ref={input} id="import-file" type="file" className="sr-only" disabled={busy} onChange={(e) => { pick(e.target.files); e.target.value = '' }} />
        <Button asChild variant="secondary" size="sm"><label htmlFor="import-file" className={busy ? 'pointer-events-none opacity-50' : 'cursor-pointer'}>{t('backups.steps.chooseBackupFile')}</label></Button>
        {busy && <p className="text-muted-foreground" role="status">{t('backups.steps.sendingFileBigFile')}</p>}
      </div>
      {local && (
        <form className="space-y-2" onSubmit={(e) => { e.preventDefault(); if (path.trim()) onPath(path.trim()) }}>
          <Field label={t('backups.steps.giveFilesPathComputer')} hint={t('backups.steps.exampleMntUsbOldserver')}><Input value={path} placeholder={t('backups.steps.pathFileTarGz')} onChange={(e) => setPath(e.target.value)} /></Field>
          <div className="flex flex-wrap gap-2">
            <Button type="submit" disabled={!path.trim() || busy}>{t('backups.steps.checkFile')}</Button>
            {inShell() && <Button type="button" disabled={busy} onClick={async () => { const f = await browseForFile(t('backups.steps.chooseBackupFile'), ['age', 'gz']); if (f) { setPath(f); onPath(f) } }}>{t('backups.steps.browse')}</Button>}
          </div>
        </form>
      )}
      {archived.length > 0 && (
        <div role="group" className="space-y-2" aria-label={t('backups.steps.backupsServersNoLonger')}>
          <p className="text-sm font-semibold">{t('backups.steps.backupsServersNoLonger')}</p>
          <p className="text-xs text-muted-foreground">{t('backups.steps.koraKeepsBackupsServer')}</p>
          <ul className="divide-y divide-border rounded-md border border-border">
            {archived.flatMap((a) => a.backups.map((b) => (
              <li key={`${a.name}-${a.deleted_unix}-${b.id}`} className="flex flex-wrap items-center gap-2 p-2 text-sm">
                <span className="font-medium">{a.name}</span>
                <span className="text-xs text-muted-foreground">{t('backups.steps.backupTolocalestringValue', { toLocaleString: new Date(b.created_unix * 1000).toLocaleString(), value: b.encrypted ? t('backups.steps.passwordProtected') : '' })}</span>
                <Button size="sm" className="ml-auto" disabled={busy} onClick={() => onArchived(a, b.id)}>{t('backups.steps.restoreNewServer')}</Button>
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
      <p className="text-sm">{rich('backups.steps.filearchiveNodeSpanNode', { fileArchive: <FileArchive className="mr-1 inline size-4" />, span: <span className="font-mono text-xs">{name}</span> })}</p>
      <Field label={t('backups.steps.itsPassword')}><Input type="password" autoComplete="off" value={pw} onChange={(e) => setPw(e.target.value)} /></Field>
      <p className="text-xs text-muted-foreground">{t('backups.steps.passwordChosenOldComputer')}</p>
      <Button type="submit" variant="default" disabled={!pw || busy}>{t('backups.steps.openFile')}</Button>
      <ErrorNotice error={error} />
    </form>
  )
}

const KIND_FIELD: Record<string, keyof ImportChoices> = { name: 'name', game_port: 'game_port', txadmin_port: 'txadmin_port' }

function ClashRow({ c, onUse }: { c: Clash; onUse: (c: Clash) => void }) {
  return (
    <li className="flex flex-wrap items-center gap-2 text-sm">
      <span>{c.message}</span>
      {c.suggestion && KIND_FIELD[c.kind] && <Button size="sm" onClick={() => onUse(c)}>{t('backups.steps.useSuggestion', { suggestion: c.suggestion })}</Button>}
      {c.kind === 'folder' && <span className="text-muted-foreground">{t('backups.steps.chooseAnotherFolderBelow')}</span>}
    </li>
  )
}

export function PreviewStep({ preview, choices, setChoices, key_, setKey, forkChoice, setForkChoice, admin, setAdmin, start, setStart, busy, onGo }: {
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
          <p><span className="font-semibold">{s.name}</span> <Badge tone="plain">{t('backups.steps.madeValue', { value: s.os === 'windows' ? 'Windows' : 'Linux' })}</Badge> <StyleChip source={s.source} versions={s.versions} /></p>
          <p className="text-muted-foreground">{t('backups.steps.referenceResourcesFetchedAgain', { by_reference: preview.by_reference, in_full: preview.in_full, value: preview.database ? ` · database: ${preview.database.kind === 'none' ? 'none' : 'included'}` : '' })}</p>
        </div>
      )}
      {preview.clashes.length > 0 && (
        <Alert tone="warning" title={t('backups.steps.somethingAlreadyUsingThese')}>
          <ul className="space-y-2">{preview.clashes.map((c) => <ClashRow key={c.kind} c={c} onUse={useClash} />)}</ul>
        </Alert>
      )}
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label={t('backups.steps.nameServerHere')}><Input value={choices.name} onChange={(e) => setChoices({ ...choices, name: e.target.value })} /></Field>
        <Field label={t('backups.steps.folderForIt')}><Input value={choices.dir} onChange={(e) => setChoices({ ...choices, dir: e.target.value })} /></Field>
        <Field label={t('backups.steps.gamePort')}><Input type="number" value={choices.game_port} onChange={(e) => setChoices({ ...choices, game_port: Number(e.target.value) })} /></Field>
        <Field label={t('backups.steps.txadminPort')} error={portBad ? t('backups.steps.useTwoDifferentPorts') : null}><Input type="number" value={choices.txadmin_port} onChange={(e) => setChoices({ ...choices, txadmin_port: Number(e.target.value) })} /></Field>
      </div>
      <div className="space-y-2" aria-label={t('backups.steps.whatCreated')}>
        <p className="text-sm font-semibold">{t('backups.steps.koraCreate')}</p>
        <ul className="list-disc space-y-1 pl-5 text-sm">{preview.will_create.map((l, i) => <li key={i}>{l}</li>)}</ul>
      </div>
      {preview.fork && !preview.fork.usable && (
        <fieldset className="space-y-2">
          <legend className="mb-1 text-sm font-semibold">{t('backups.steps.copiesGithubOwner', { owner: preview.fork.owner })}</legend>
          <Alert tone="warning">{preview.fork.message}</Alert>
          <Radio name="fork" checked={forkChoice === 'keep'} onChange={() => setForkChoice('keep')} label={t('backups.steps.keepRemotesRecorded')} hint={t('backups.steps.resourcesFetchedFromOld')} />
          <Radio name="fork" checked={forkChoice === 'own_copies'} onChange={() => setForkChoice('own_copies')} disabled={!preview.fork.signed_in_as} label={t('backups.steps.makeMyOwnCopies')} hint={preview.fork.signed_in_as ? t('backups.steps.koraForksOriginalRepositories', { signed_in_as: preview.fork.signed_in_as }) : t('backups.steps.signGithubComputerFirst')} />
          <Radio name="fork" checked={forkChoice === 'originals'} onChange={() => setForkChoice('originals')} label={t('backups.steps.useOriginals')} hint={t('backups.steps.plainCopiesOriginalRepositorie')} />
        </fieldset>
      )}
      {preview.db_login && !preview.db_login.ok && <DbLoginForm note={preview.db_login.message} setAdmin={setAdmin} />}
      <fieldset className="space-y-2">
        <legend className="mb-1 text-sm font-semibold">{t('backups.steps.licenceKey')}</legend>
        <Alert tone="warning">{preview.licence_note}</Alert>
        <Radio name="key" checked={key_ === null} onChange={() => setKey(null)} label={t('backups.steps.useLicenceKeyFile')} />
        {preview.saved_keys.length > 0 && (
          <Radio name="key" checked={key_ !== null} onChange={() => setKey(preview.saved_keys[0])} label={t('backups.steps.useOneMySaved')} hint={key_ !== null ? (
            <select aria-label={t('backups.steps.savedLicenceKey')} className="mt-1 h-9 rounded-md border border-input bg-ink px-3 text-sm" value={key_} onChange={(e) => setKey(e.target.value)}>{preview.saved_keys.map((k) => <option key={k}>{k}</option>)}</select>
          ) : undefined} />
        )}
      </fieldset>
      <div className="space-y-2">
        <p className="text-sm font-semibold">{t('backups.steps.whatDoesComeAlong')}</p>
        <ul className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">{preview.wont_carry.map((l, i) => <li key={i}>{l}</li>)}</ul>
      </div>
      {preview.warnings.map((w, i) => <Alert key={i} tone="info">{w}</Alert>)}
      <Checkbox checked={start} onChange={(e) => setStart(e.target.checked)} label={t('backups.steps.startServerWhenReady')} />
      <Button variant="default" disabled={busy || portBad || !choices.name.trim() || !choices.dir.trim() || (preview.db_login?.ok === false && !admin)} onClick={onGo}>{t('backups.steps.bringServer')}</Button>
    </div>
  )
}

/** The database administrator login, when the automatic one does not work: the same choice the install wizard's database step offers. Used once, never stored. */
function DbLoginForm({ note, setAdmin }: { note: string; setAdmin: (a: DbAdmin | null) => void }) {
  const [f, setF] = useState({ host: '127.0.0.1', port: 3306, user: 'root', password: '' })
  const test = useMutation({ mutationFn: () => api.installs.checkDbLogin({ host: f.host, port: f.port, user: f.user, password: f.password }) })
  return (
    <fieldset className="space-y-3">
      <legend className="mb-1 text-sm font-semibold">{t('backups.steps.databaseAdministratorLogin')}</legend>
      <Alert tone="warning">{t('backups.steps.noteKoraNeedsAdministrator', { note })}</Alert>
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label={t('backups.steps.databaseServerAddress')}><Input value={f.host} onChange={(e) => { setF({ ...f, host: e.target.value }); setAdmin(null) }} /></Field>
        <Field label={t('backups.steps.port')}><Input type="number" value={f.port} onChange={(e) => { setF({ ...f, port: Number(e.target.value) }); setAdmin(null) }} /></Field>
        <Field label={t('backups.steps.administratorUser')}><Input value={f.user} onChange={(e) => { setF({ ...f, user: e.target.value }); setAdmin(null) }} /></Field>
        <Field label={t('backups.steps.administratorPassword')}><Input type="password" autoComplete="off" value={f.password} onChange={(e) => { setF({ ...f, password: e.target.value }); setAdmin(null) }} /></Field>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" disabled={test.isPending || !f.user} onClick={() => test.mutate(undefined, { onSuccess: (r) => setAdmin(r.ok ? { host: f.host, port: f.port, user: f.user, password: f.password } : null) })}>{t('backups.steps.testLogin')}</Button>
        {test.data && <span className={`text-sm ${test.data.ok ? 'text-success' : 'text-danger-text'}`} role="status">{test.data.message}</span>}
      </div>
      <ErrorNotice error={test.error} />
    </fieldset>
  )
}

export function RunStep({ events, failed, onRetry, onStartOver }: { events: BackupEvent[]; failed: JobError | null; onRetry: () => void; onStartOver: () => void }) {
  const last = events.at(-1)
  return (
    <div className="space-y-4">
      <Announcer message={failed ? '' : progressSentence(t('backups.steps.importProgress'), last?.percent ?? 0)} />
      {!failed && (
        <>
          <Progress value={last?.percent ?? 0} label={t('backups.steps.importProgress')} />
          <p className="text-sm text-muted-foreground">{last?.message ?? t('backups.steps.starting')}</p>
        </>
      )}
      {failed && (
        <>
          <ErrorNotice error={failed} onRetry={onRetry} />
          <div className="flex flex-wrap gap-2">
            <Button variant="default" onClick={onRetry}>{t('backups.steps.carryFromWhereStopped')}</Button>
            <Button variant="outline" onClick={onStartOver}>{t('backups.steps.startOverRemovesWhat')}</Button>
          </div>
        </>
      )}
    </div>
  )
}

export function DoneStep({ r }: { r: ImportResult }) {
  return (
    <div className="space-y-4">
      <Alert tone={r.problems.length ? 'warning' : 'success'} title={t('backups.steps.nameImported', { name: r.name })}>
        <p>{rich('backups.steps.spanNodeValue', { span: <span className="font-mono text-xs">{r.dir}</span>, value: r.started ? ' and it was started' : '' })}</p>
        {r.resources_fetched.length > 0 && <ul className="mt-1 list-disc pl-5">{r.resources_fetched.map((m, i) => <li key={i}>{m}</li>)}</ul>}
        {r.notes.map((n, i) => <p key={i} className="mt-1 text-muted-foreground">{n}</p>)}
      </Alert>
      {r.problems.length > 0 && <Alert tone="danger" title={t('backups.steps.whatDidWork')}><ul className="list-disc pl-5">{r.problems.map((m, i) => <li key={i}>{m}</li>)}</ul></Alert>}
      <div className="space-y-1">
        <p className="text-sm font-semibold">{t('backups.steps.nextComputer')}</p>
        <ul className="list-disc space-y-1 pl-5 text-sm">{r.next_steps.map((m, i) => <li key={i}>{m}</li>)}</ul>
      </div>
      <div className="space-y-1">
        <p className="text-sm font-semibold">{t('backups.steps.removeFromOldComputer')}</p>
        <p className="text-sm text-muted-foreground">{t('backups.steps.koraDoesReachInto')}</p>
        <ol className="list-decimal space-y-1 pl-5 text-sm">{r.old_machine_steps.map((m, i) => <li key={i}>{m}</li>)}</ol>
      </div>
      <Button asChild variant="default"><Link to={`/servers/${encodeURIComponent(r.name)}`}>{t('backups.steps.openName', { name: r.name })}</Link></Button>
    </div>
  )
}

export type { ImportSource }
