import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Download, Lock, ShieldCheck, Trash2, Undo2 } from 'lucide-react'
import { api } from '@/api/client'
import type { BackupCreated, BackupEvent, BackupInfo, BackupSettings, RestorePreview, RestoreReport, ServerDetail, VerifyResult } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox, Radio } from '@/components/ui/checkbox'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Input, Select } from '@/components/ui/input'
import { Progress } from '@/components/ui/progress'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { CopyField } from '@/components/CopyButton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { timeAgo } from '@/lib/ago'
import { downloadFile } from '@/lib/download'
import { fileSize } from './PaidAssetsTab'

const DAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday']
const MIN_PASSWORD = 8

/** What the backup file holds. Said before and after every backup, in plain words. */
export const SECRETS_NOTE = 'A backup holds your secrets: the licence key, the database login and the txAdmin admin. Keep the file private. Kora makes it readable only by your account here, and only an administrator can download it.'

function when(unix: number): string {
  return `${new Date(unix * 1000).toLocaleString()} (${timeAgo(unix * 1000, Date.now())})`
}

function latestPercent(events: BackupEvent[]): { percent: number; message: string } {
  const e = events.at(-1)
  return { percent: e?.percent ?? 0, message: e?.message ?? 'Starting…' }
}

function JobBar({ events, label }: { events: BackupEvent[]; label: string }) {
  const { percent, message } = latestPercent(events)
  return (
    <div className="space-y-1.5" aria-live="polite">
      <Progress value={percent} label={label} />
      <p className="text-sm text-muted-foreground">{message}</p>
    </div>
  )
}

// ---- make a backup ---------------------------------------------------------------------------------------------

function MakeBackup({ server: s, onDone }: { server: ServerDetail; onDone: () => void }) {
  const [protect, setProtect] = useState(false)
  const [pw, setPw] = useState('')
  const [pw2, setPw2] = useState('')
  const [stop, setStop] = useState(false)
  const [everything, setEverything] = useState(false)
  const [job, setJob] = useState<string | null>(null)
  const [err, setErr] = useState<unknown>(null)
  const run = useJob<BackupCreated, BackupEvent>(job, () => onDone())
  const busy = run.phase === 'running'
  const pwProblem = protect && (pw.length < MIN_PASSWORD ? `Use at least ${MIN_PASSWORD} characters.` : pw !== pw2 ? 'The two passwords are not the same.' : null)
  const start = async () => {
    setErr(null)
    try {
      setJob((await api.backups.create(s.name, { passphrase: protect ? pw : undefined, everything, stop_server: stop && s.running, for_move: false })).job)
    } catch (e) {
      setErr(e)
    }
  }
  return (
    <Card>
      <CardHeader><CardTitle>Back up now</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <Alert tone="info">{SECRETS_NOTE}</Alert>
        <Checkbox
          checked={protect}
          onChange={(e) => setProtect(e.target.checked)}
          disabled={busy}
          label="Protect the file with a password"
          hint="Recommended when the file leaves this computer (a USB stick, a cloud drive, an email). Without one, the file is only as private as its folder."
        />
        {protect && (
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Password" error={pw && pwProblem ? pwProblem : null}><Input type="password" autoComplete="new-password" value={pw} onChange={(e) => setPw(e.target.value)} disabled={busy} /></Field>
            <Field label="Password again"><Input type="password" autoComplete="new-password" value={pw2} onChange={(e) => setPw2(e.target.value)} disabled={busy} /></Field>
            <Alert tone="warning" className="sm:col-span-2">Kora does not keep this password and nobody can recover it. If you lose it, this backup cannot be opened. Write it down somewhere safe.</Alert>
          </div>
        )}
        <Checkbox
          checked={stop}
          onChange={(e) => setStop(e.target.checked)}
          disabled={busy || !s.running}
          label="Stop the server while the backup runs"
          hint={s.running ? 'The copy is then perfectly consistent, but players are disconnected. The server starts again afterwards. Without this, the database is one consistent snapshot and the other files are copied one by one while the server runs.' : 'The server is stopped, so the copy is consistent already.'}
        />
        <Checkbox checked={everything} onChange={(e) => setEverything(e.target.checked)} disabled={busy} label="Keep every resource in full" hint="A bigger file that does not need the internet to restore. Normally resources are saved as a reference (where they come from and the exact version) and fetched again; only changed, kept-aside and paid ones are kept in full." />
        <div className="flex flex-wrap items-center gap-3">
          <Button variant="default" disabled={busy || !!pwProblem} onClick={() => void start()}>{busy ? 'Backing up…' : 'Back up now'}</Button>
        </div>
        {busy && <JobBar events={run.events} label="Backup progress" />}
        <ErrorNotice error={err ?? (run.phase === 'failed' ? run.error : null)} />
        {run.phase === 'done' && run.result && (
          <Alert tone="success" title="Backup finished">
            <p>{run.result.info.file} · {fileSize(run.result.info.bytes)} · {run.result.info.by_reference} resources by reference, {run.result.info.in_full} in full.</p>
            <p className="mt-1">Kept in <span className="font-mono text-xs">{run.result.path}</span></p>
            {run.result.info.encrypted && <p className="mt-1 font-medium">Keep the password safe: it cannot be recovered.</p>}
            {run.result.notes.map((n, i) => <p key={i} className="mt-1 text-muted-foreground">{n}</p>)}
          </Alert>
        )}
      </CardContent>
    </Card>
  )
}

// ---- check / restore -------------------------------------------------------------------------------------------

function PasswordField({ value, onChange, error }: { value: string; onChange: (v: string) => void; error?: string | null }) {
  return <Field label="This backup's password" error={error}><Input type="password" autoComplete="off" value={value} onChange={(e) => onChange(e.target.value)} /></Field>
}

function VerifyDialog({ server, backup, onClose }: { server: string; backup: BackupInfo; onClose: () => void }) {
  const [pw, setPw] = useState('')
  const [job, setJob] = useState<string | null>(null)
  const [err, setErr] = useState<unknown>(null)
  const run = useJob<VerifyResult, BackupEvent>(job)
  const start = async () => {
    setErr(null)
    try {
      setJob((await api.backups.verify(server, backup.id, pw || undefined)).job)
    } catch (e) {
      setErr(e)
    }
  }
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent title="Check this backup" description="Kora opens the whole file and compares every file inside with its checksum. Nothing on the server changes.">
        <div className="space-y-4">
          {backup.encrypted && run.phase === 'idle' && <PasswordField value={pw} onChange={setPw} />}
          {run.phase === 'running' && <JobBar events={run.events} label="Check progress" />}
          <ErrorNotice error={err ?? (run.phase === 'failed' ? run.error : null)} />
          {run.phase === 'done' && run.result && <Alert tone="success" title="This backup is good">{run.result.summary}</Alert>}
          <div className="flex justify-end gap-2">
            <Button onClick={onClose}>Close</Button>
            {run.phase !== 'done' && <Button variant="default" disabled={run.phase === 'running' || (backup.encrypted && !pw)} onClick={() => void start()}>Check it</Button>}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function Report({ r }: { r: RestoreReport }) {
  return (
    <div className="space-y-3">
      <Alert tone={r.problems.length ? 'warning' : 'success'} title={r.problems.length ? 'Restored, with some things to look at' : 'Restore finished'}>
        <p>{r.database_restored ? 'The database was put back. ' : ''}{r.files_restored} files were put back.{r.restarted ? ' The server was started again.' : ''}</p>
        {r.resources_restored.length > 0 && <ul className="mt-1 list-disc pl-5">{r.resources_restored.map((m, i) => <li key={i}>{m}</li>)}</ul>}
        {r.notes.map((m, i) => <p key={i} className="mt-1 text-muted-foreground">{m}</p>)}
      </Alert>
      {r.problems.length > 0 && <Alert tone="danger" title="What did not work"><ul className="list-disc pl-5">{r.problems.map((m, i) => <li key={i}>{m}</li>)}</ul></Alert>}
    </div>
  )
}

function RestoreDialog({ server, backup, canRestore, why, onClose, onChanged }: { server: ServerDetail; backup: BackupInfo; canRestore: boolean; why: string | null; onClose: () => void; onChanged: () => void }) {
  const [pw, setPw] = useState(backup.encrypted ? '' : undefined as string | undefined)
  const [preview, setPreview] = useState<RestorePreview | null>(null)
  const [job, setJob] = useState<string | null>(null)
  const [err, setErr] = useState<unknown>(null)
  const look = useMutation({
    mutationFn: () => api.backups.preview(server.name, backup.id, pw || undefined),
    onSuccess: (p) => { setPreview(p); setErr(null) },
    onError: (e) => setErr(e),
  })
  // An unprotected backup is previewed at once; a protected one waits for its password.
  useEffect(() => {
    if (!backup.encrypted) look.mutate()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  const run = useJob<RestoreReport, BackupEvent>(job, () => onChanged())
  const start = async () => {
    setErr(null)
    try {
      setJob((await api.backups.restore(server.name, backup.id, pw || undefined)).job)
    } catch (e) {
      setErr(e)
    }
  }
  const ready = preview && !preview.needs_passphrase
  return (
    <Dialog open onOpenChange={(o) => !o && run.phase !== 'running' && onClose()}>
      <DialogContent title="Restore this backup" description={`Backup of ${when(backup.created_unix)}`}>
        <div className="space-y-4">
          {why && <Alert tone="info">{why}</Alert>}
          {backup.encrypted && !ready && (
            <div className="space-y-3">
              <PasswordField value={pw ?? ''} onChange={setPw} />
              <Button variant="default" disabled={!pw || look.isPending} onClick={() => look.mutate()}>Open the backup</Button>
            </div>
          )}
          {look.isPending && <p className="text-sm text-muted-foreground">Reading the backup…</p>}
          {ready && run.phase === 'idle' && (
            <div className="space-y-3" aria-label="What a restore will do">
              <p className="text-sm font-semibold">This will:</p>
              <ul className="list-disc space-y-1 pl-5 text-sm">{preview.will_change.map((l, i) => <li key={i}>{l}</li>)}</ul>
              <p className="text-sm font-semibold">This stays as it is:</p>
              <ul className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">{preview.will_keep.map((l, i) => <li key={i}>{l}</li>)}</ul>
              {preview.warnings.map((w, i) => <Alert key={i} tone="warning">{w}</Alert>)}
            </div>
          )}
          {run.phase === 'running' && <JobBar events={run.events} label="Restore progress" />}
          <ErrorNotice error={err ?? (run.phase === 'failed' ? run.error : null)} />
          {run.phase === 'done' && run.result && <Report r={run.result} />}
          <div className="flex justify-end gap-2">
            <Button disabled={run.phase === 'running'} onClick={onClose}>{run.phase === 'done' ? 'Close' : 'Cancel'}</Button>
            {ready && run.phase === 'idle' && <Button variant="destructive" disabled={!canRestore} title={why ?? undefined} onClick={() => void start()}>Restore now</Button>}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function UndoBanner({ server, undo, onChanged, canRestore }: { server: string; undo: { id: string; created_unix: number; encrypted: boolean }; onChanged: () => void; canRestore: boolean }) {
  const [open, setOpen] = useState(false)
  const [pw, setPw] = useState('')
  const [job, setJob] = useState<string | null>(null)
  const [err, setErr] = useState<unknown>(null)
  const run = useJob<RestoreReport, BackupEvent>(job, () => onChanged())
  const start = async () => {
    setErr(null)
    try {
      setJob((await api.backups.undo(server, pw || undefined)).job)
    } catch (e) {
      setErr(e)
    }
  }
  if (run.phase === 'done' && run.result) return <Report r={run.result} />
  return (
    <Alert tone="info" title="The last restore can be undone">
      <p>Just before it, Kora saved the server as it was ({when(undo.created_unix)}). You can put that back.</p>
      {!open ? (
        <Button className="mt-2" size="sm" disabled={!canRestore} onClick={() => setOpen(true)}><Undo2 /> Undo this restore</Button>
      ) : (
        <div className="mt-3 space-y-3">
          {undo.encrypted && <PasswordField value={pw} onChange={setPw} />}
          {run.phase === 'running' ? <JobBar events={run.events} label="Undo progress" /> : <Button variant="destructive" size="sm" disabled={undo.encrypted && !pw} onClick={() => void start()}>Put the server back as it was</Button>}
          <ErrorNotice error={err ?? (run.phase === 'failed' ? run.error : null)} />
        </div>
      )}
    </Alert>
  )
}

// ---- the list --------------------------------------------------------------------------------------------------

const LABEL_TEXT: Record<BackupInfo['label'], string> = { manual: 'Made by you', scheduled: 'Scheduled', before_restore: 'Safety copy before a restore', move: 'For moving the server' }

function Row({ server, b, canAdmin, remote, onVerify, onRestore, onChanged }: { server: string; b: BackupInfo; canAdmin: boolean; remote: boolean; onVerify: () => void; onRestore: () => void; onChanged: () => void }) {
  const [confirm, setConfirm] = useState(false)
  const del = useMutation({ mutationFn: () => api.backups.remove(server, b.id), onSuccess: () => { setConfirm(false); onChanged() } })
  const dl = useMutation({ mutationFn: () => downloadFile(api.backups.fileUrl(server, b.id), b.file, b.bytes) })
  return (
    <li className="space-y-2 p-3 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{when(b.created_unix)}</span>
        <Badge tone="plain">{fileSize(b.bytes)}</Badge>
        {b.encrypted ? <Badge tone="success"><Lock className="size-3" /> Password protected</Badge> : <Badge tone="neutral">Not password protected</Badge>}
        <Badge tone="plain">{LABEL_TEXT[b.label]}</Badge>
        {b.stopped && <Badge tone="plain">Server stopped</Badge>}
        {b.too_new && <Badge tone="danger">Made by a newer Kora</Badge>}
        <span className="ml-auto flex flex-wrap gap-2">
          <Button size="sm" onClick={onVerify}><ShieldCheck /> Check</Button>
          <Button size="sm" variant="outline" disabled={!canAdmin || b.too_new} onClick={onRestore}>Restore…</Button>
          {remote && <Button size="sm" disabled={!canAdmin || dl.isPending} onClick={() => dl.mutate()}><Download /> Download</Button>}
          <Button size="sm" variant="outline" disabled={!canAdmin} onClick={() => setConfirm(true)} aria-label={`Delete the backup of ${when(b.created_unix)}`}><Trash2 /></Button>
        </span>
      </div>
      <p className="text-xs text-muted-foreground">{b.by_reference} resources by reference, {b.in_full} in full · database: {b.database === 'dumped' ? 'included' : b.database === 'external' ? 'included (on another computer)' : b.database === 'none' ? 'none to back up' : 'not included'} · Kora {b.kora_version}</p>
      <ErrorNotice error={del.error ?? dl.error} />
      <ConfirmDialog open={confirm} onOpenChange={setConfirm} title="Delete this backup?" description="The file is removed from the disk. It cannot be brought back." confirmLabel="Delete" destructive busy={del.isPending} onConfirm={() => del.mutate()} />
    </li>
  )
}

// ---- settings --------------------------------------------------------------------------------------------------

function Settings({ server, view, canChange, why, onSaved }: { server: string; view: NonNullable<ReturnType<typeof useBackupsView>['data']>; canChange: boolean; why: string | null; onSaved: () => void }) {
  const [s, setS] = useState<BackupSettings>(view.settings)
  useEffect(() => setS(view.settings), [view.settings])
  const save = useMutation({ mutationFn: () => api.backups.setSettings(server, s), onSuccess: onSaved })
  const prune = useMutation({ mutationFn: () => api.backups.prune(server), onSuccess: onSaved })
  const sch = s.schedule
  const setSch = (p: Partial<BackupSettings['schedule']>) => setS({ ...s, schedule: { ...sch, ...p } })
  const badTime = sch.enabled && !/^([01]\d|2[0-3]):[0-5]\d$/.test(sch.time)
  return (
    <Card>
      <CardHeader><CardTitle>Where backups go, and when</CardTitle></CardHeader>
      <CardContent className="space-y-5">
        {why && <Alert tone="info">{why}</Alert>}
        <div className="space-y-2">
          <div className="text-sm font-semibold">Backups are kept in</div>
          <CopyField value={view.folder} label="Backups folder" />
          <p className="text-xs text-muted-foreground">{view.default_folder ? "This is Kora's own backups folder. It is outside the server's folder, so deleting the server does not delete its backups, but it is on the same disk as Kora. " : ''}For a backup that survives a dead disk, keep one on another disk or a mounted share.{view.free_bytes != null ? ` ${fileSize(view.free_bytes)} free.` : ''}</p>
          <Field label="Use another folder (full path; leave empty for Kora's own)"><Input value={s.folder} disabled={!canChange} placeholder="/mnt/backups or D:\Backups" onChange={(e) => setS({ ...s, folder: e.target.value })} /></Field>
        </div>
        <fieldset className="space-y-2">
          <legend className="mb-1 text-sm font-semibold">Scheduled backups</legend>
          <Radio name="when" checked={!sch.enabled} disabled={!canChange} onChange={() => setSch({ enabled: false })} label="Off" hint="Backups only happen when you press the button." />
          <Radio name="when" checked={sch.enabled && sch.cadence === 'daily'} disabled={!canChange} onChange={() => setSch({ enabled: true, cadence: 'daily', weekday: undefined })} label="Every day" />
          <Radio name="when" checked={sch.enabled && sch.cadence === 'weekly'} disabled={!canChange} onChange={() => setSch({ enabled: true, cadence: 'weekly', weekday: sch.weekday ?? 6 })} label="Once a week" />
        </fieldset>
        {sch.enabled && (
          <div className="space-y-3">
            <div className="flex flex-wrap gap-4">
              {sch.cadence === 'weekly' && <div className="w-44"><Field label="Day"><Select value={sch.weekday ?? 6} disabled={!canChange} onChange={(e) => setSch({ weekday: Number(e.target.value) })}>{DAYS.map((d, i) => <option key={d} value={i}>{d}</option>)}</Select></Field></div>}
              <div className="w-32"><Field label="Time (24 hours)" error={badTime ? 'Use the form HH:MM, for example 03:30.' : null}><Input value={sch.time} disabled={!canChange} onChange={(e) => setSch({ time: e.target.value })} inputMode="numeric" placeholder="03:30" /></Field></div>
            </div>
            <Checkbox checked={s.stop_for_scheduled} disabled={!canChange} onChange={(e) => setS({ ...s, stop_for_scheduled: e.target.checked })} label="Stop the server during scheduled backups" hint="A perfectly consistent copy, but players are disconnected at that time." />
            <Alert tone="info">{view.schedule_note}</Alert>
          </div>
        )}
        <fieldset className="space-y-2">
          <legend className="mb-1 text-sm font-semibold">How many to keep</legend>
          <div className="flex flex-wrap gap-4">
            <div className="w-44"><Field label="Newest backups (0 = all)"><Input type="number" min={0} max={1000} value={s.retention.keep_last} disabled={!canChange} onChange={(e) => setS({ ...s, retention: { ...s.retention, keep_last: Math.max(0, Number(e.target.value) || 0) } })} /></Field></div>
            <div className="w-56"><Field label="Also one per day for this many days"><Input type="number" min={0} max={3650} value={s.retention.keep_daily_days} disabled={!canChange} onChange={(e) => setS({ ...s, retention: { ...s.retention, keep_daily_days: Math.max(0, Number(e.target.value) || 0) } })} /></Field></div>
          </div>
          <p className="text-xs text-muted-foreground">Older ones are removed after each backup. A copy made to move the server, and the safety copy a restore can be undone from, are not removed.</p>
        </fieldset>
        <div className="flex flex-wrap gap-2">
          <Button variant="default" disabled={!canChange || save.isPending || badTime} title={why ?? undefined} onClick={() => save.mutate()}>Save</Button>
          <Button disabled={!canChange || prune.isPending} onClick={() => prune.mutate()}>Remove old backups now</Button>
        </div>
        {save.isSuccess && <Alert tone="success">Saved.</Alert>}
        {prune.data && <Alert tone="success">{prune.data.removed.length ? `Removed ${prune.data.removed.length} old backup${prune.data.removed.length === 1 ? '' : 's'}.` : 'Nothing to remove.'}</Alert>}
        <ErrorNotice error={save.error ?? prune.error} />
      </CardContent>
    </Card>
  )
}

// ---- the tab ---------------------------------------------------------------------------------------------------

function useBackupsView(name: string, enabled: boolean) {
  return useQuery({ queryKey: ['backups', name], queryFn: () => api.backups.list(name), enabled })
}

export function BackupsTab({ server: s }: { server: ServerDetail }) {
  const qc = useQueryClient()
  const role = useRole()
  const noList = whyNot(role, 'operator')
  const whyAdmin = whyNot(role, 'admin', true)
  const view = useBackupsView(s.name, !noList)
  const [verify, setVerify] = useState<BackupInfo | null>(null)
  const [restore, setRestore] = useState<BackupInfo | null>(null)
  const changed = () => {
    void qc.invalidateQueries({ queryKey: ['backups', s.name] })
    void qc.invalidateQueries({ queryKey: ['backups-overview'] })
  }
  if (noList) return <Alert tone="info">{noList}</Alert>
  const v = view.data
  return (
    <div className="grid gap-5 lg:grid-cols-2">
      <div className="space-y-5">
        <ErrorNotice error={view.error} />
        {v && (
          <Card>
            <CardHeader><CardTitle>{v.summary.text}</CardTitle></CardHeader>
            <CardContent className="space-y-3 text-sm">
              {v.summary.warning && <Alert tone="warning">{v.summary.warning}</Alert>}
              <p className="text-muted-foreground">{v.database.note}</p>
              {v.database.tool_missing && <Alert tone="warning" title="The database tools are missing">Kora needs mariadb-dump (or mysqldump) to copy the database. Install the MariaDB client tools, then try again.</Alert>}
            </CardContent>
          </Card>
        )}
        {v?.undo && <UndoBanner server={s.name} undo={v.undo} onChanged={changed} canRestore={!whyAdmin} />}
        <MakeBackup server={s} onDone={changed} />
        <Card>
          <CardHeader><CardTitle>Your backups{v ? ` (${v.backups.length})` : ''}</CardTitle></CardHeader>
          <CardContent>
            {v && v.backups.length === 0 && <p className="text-sm text-muted-foreground">No backup yet. Press "Back up now": it takes a minute or two.</p>}
            {v && v.backups.length > 0 && (
              <ul className="divide-y divide-border rounded-md border border-border">
                {v.backups.map((b) => <Row key={b.id} server={s.name} b={b} canAdmin={!whyAdmin} remote={role.remote} onVerify={() => setVerify(b)} onRestore={() => setRestore(b)} onChanged={changed} />)}
              </ul>
            )}
            {whyAdmin && v && v.backups.length > 0 && <p className="mt-2 text-xs text-muted-foreground">{whyAdmin}</p>}
          </CardContent>
        </Card>
      </div>
      <div className="space-y-5">{v && <Settings server={s.name} view={v} canChange={!whyAdmin} why={whyAdmin} onSaved={changed} />}</div>
      {verify && <VerifyDialog server={s.name} backup={verify} onClose={() => setVerify(null)} />}
      {restore && <RestoreDialog server={s} backup={restore} canRestore={!whyAdmin} why={whyAdmin} onClose={() => setRestore(null)} onChanged={changed} />}
    </div>
  )
}
