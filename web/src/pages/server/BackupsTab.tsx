import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Download, Lock, ShieldCheck, Trash2, Undo2 } from 'lucide-react'
import { api } from '@/api/client'
import type { BackupCreated, BackupEvent, BackupInfo, BackupSettings, BackupsView, RestorePreview, RestoreReport, ServerDetail, VerifyResult } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox, Radio } from '@/components/ui/checkbox'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Input, Select } from '@/components/ui/input'
import { Announcer, progressSentence } from '@/components/Announcer'
import { Progress } from '@/components/ui/progress'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { CopyField } from '@/components/CopyButton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { timeAgo } from '@/lib/ago'
import { downloadFile } from '@/lib/download'
import { browseForFolder, inShell } from '@/lib/shell'
import { BackupSuggestion } from '@/components/BackupSuggestion'
import { fileSize } from './PaidAssetsTab'
import { t, msg, type Msg, unix as unixToDate } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/backups'

const DAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday']
const MIN_PASSWORD = 8

/** What the backup file holds. Said before and after every backup, in plain words. */
export const SECRETS_NOTE = msg('backups.backupsTab.backupHoldsSecretsLicence')

function when(seconds: number): string {
  return t('backups.backupsTab.whenAgo', { when: unixToDate(seconds), ago: timeAgo(seconds * 1000, Date.now()) })
}

function latestPercent(events: BackupEvent[]): { percent: number; message: string } {
  const e = events.at(-1)
  return { percent: e?.percent ?? 0, message: e?.message ?? 'Starting…' }
}

function JobBar({ events, label }: { events: BackupEvent[]; label: string }) {
  const { percent, message } = latestPercent(events)
  return (
    <div className="space-y-1.5">
      <Announcer message={progressSentence(label, percent)} />
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
  const pwProblem = protect && (pw.length < MIN_PASSWORD ? t('backups.backupsTab.useLeastMinPassword', { MIN_PASSWORD }) : pw !== pw2 ? t('backups.backupsTab.twoPasswordsSame') : null)
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
      <CardHeader><CardTitle>{t('backups.backupsTab.backUpNow')}</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <Alert tone="info">{t(SECRETS_NOTE)}</Alert>
        <Checkbox
          checked={protect}
          onChange={(e) => setProtect(e.target.checked)}
          disabled={busy}
          label={t('backups.backupsTab.protectFilePassword')}
          hint={t('backups.backupsTab.recommendedWhenFileLeaves')}
        />
        {protect && (
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label={t('backups.backupsTab.password')} error={pw && pwProblem ? pwProblem : null}><Input type="password" autoComplete="new-password" value={pw} onChange={(e) => setPw(e.target.value)} disabled={busy} /></Field>
            <Field label={t('backups.backupsTab.passwordAgain')}><Input type="password" autoComplete="new-password" value={pw2} onChange={(e) => setPw2(e.target.value)} disabled={busy} /></Field>
            <Alert tone="warning" className="sm:col-span-2">{t('backups.backupsTab.koraDoesKeepPassword')}</Alert>
          </div>
        )}
        <Checkbox
          checked={stop}
          onChange={(e) => setStop(e.target.checked)}
          disabled={busy || !s.running}
          label={t('backups.backupsTab.stopServerWhileBackup')}
          hint={s.running ? t('backups.backupsTab.copyPerfectlyConsistentBut') : t('backups.backupsTab.serverStoppedCopyConsistent')}
        />
        <Checkbox checked={everything} onChange={(e) => setEverything(e.target.checked)} disabled={busy} label={t('backups.backupsTab.keepEveryResourceFull')} hint={t('backups.backupsTab.biggerFileDoesNeed')} />
        <div className="flex flex-wrap items-center gap-3">
          <Button variant="default" disabled={busy || !!pwProblem} onClick={() => void start()}>{busy ? t('backups.backupsTab.backingUp') : t('backups.backupsTab.backUpNow')}</Button>
        </div>
        {busy && <JobBar events={run.events} label={t('backups.backupsTab.backupProgress')} />}
        <ErrorNotice error={err ?? (run.phase === 'failed' ? run.error : null)} />
        {run.phase === 'done' && run.result && (
          <Alert tone="success" title={t('backups.backupsTab.backupFinished')}>
            <p>{t('backups.backupsTab.fileFilesizeReferenceResources', { file: run.result.info.file, fileSize: fileSize(run.result.info.bytes), by_reference: run.result.info.by_reference, in_full: run.result.info.in_full })}</p>
            <p className="mt-1">{rich('backups.backupsTab.keptSpanNode', { span: <span className="font-mono text-xs">{run.result.path}</span> })}</p>
            {run.result.info.encrypted && <p className="mt-1 font-medium">{t('backups.backupsTab.keepPasswordSafeCannot')}</p>}
            {run.result.notes.map((n, i) => <p key={i} className="mt-1 text-muted-foreground">{n}</p>)}
          </Alert>
        )}
      </CardContent>
    </Card>
  )
}

// ---- check / restore -------------------------------------------------------------------------------------------

function PasswordField({ value, onChange, error }: { value: string; onChange: (v: string) => void; error?: string | null }) {
  return <Field label={t('backups.backupsTab.backupsPassword')} error={error}><Input type="password" autoComplete="off" value={value} onChange={(e) => onChange(e.target.value)} /></Field>
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
      <DialogContent title={t('backups.backupsTab.checkBackup')} description={t('backups.backupsTab.koraOpensWholeFile')}>
        <div className="space-y-4">
          {backup.encrypted && run.phase === 'idle' && <PasswordField value={pw} onChange={setPw} />}
          {run.phase === 'running' && <JobBar events={run.events} label={t('backups.backupsTab.checkProgress')} />}
          <ErrorNotice error={err ?? (run.phase === 'failed' ? run.error : null)} />
          {run.phase === 'done' && run.result && <Alert tone="success" title={t('backups.backupsTab.backupGood')}>{run.result.summary}</Alert>}
          <div className="flex justify-end gap-2">
            <Button onClick={onClose}>{t('backups.backupsTab.close')}</Button>
            {run.phase !== 'done' && <Button variant="default" disabled={run.phase === 'running' || (backup.encrypted && !pw)} onClick={() => void start()}>{t('backups.backupsTab.checkIt')}</Button>}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function Report({ r }: { r: RestoreReport }) {
  return (
    <div className="space-y-3">
      <Alert tone={r.problems.length ? 'warning' : 'success'} title={r.problems.length ? t('backups.backupsTab.restoredSomeThingsLook') : t('backups.backupsTab.restoreFinished')}>
        <p>{t('backups.backupsTab.valueFilesRestoredFiles', { value: r.database_restored ? t('backups.backupsTab.databasePutBack') : '', files_restored: r.files_restored, value2: r.restarted ? t('backups.backupsTab.serverStartedAgain') : '' })}</p>
        {r.resources_restored.length > 0 && <ul className="mt-1 list-disc pl-5">{r.resources_restored.map((m, i) => <li key={i}>{m}</li>)}</ul>}
        {r.notes.map((m, i) => <p key={i} className="mt-1 text-muted-foreground">{m}</p>)}
      </Alert>
      {r.problems.length > 0 && <Alert tone="danger" title={t('backups.backupsTab.whatDidWork')}><ul className="list-disc pl-5">{r.problems.map((m, i) => <li key={i}>{m}</li>)}</ul></Alert>}
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
      <DialogContent title={t('backups.backupsTab.restoreBackup')} description={t('backups.backupsTab.backupWhen', { when: when(backup.created_unix) })}>
        <div className="space-y-4">
          {why && <Alert tone="info">{why}</Alert>}
          {backup.encrypted && !ready && (
            <div className="space-y-3">
              <PasswordField value={pw ?? ''} onChange={setPw} />
              <Button variant="default" disabled={!pw || look.isPending} onClick={() => look.mutate()}>{t('backups.backupsTab.openBackup')}</Button>
            </div>
          )}
          {look.isPending && <p className="text-sm text-muted-foreground">{t('backups.backupsTab.readingBackup')}</p>}
          {ready && run.phase === 'idle' && (
            <div className="space-y-3" aria-label={t('backups.backupsTab.whatRestoreDo')}>
              <p className="text-sm font-semibold">{t('backups.backupsTab.thisWill')}</p>
              <ul className="list-disc space-y-1 pl-5 text-sm">{preview.will_change.map((l, i) => <li key={i}>{l}</li>)}</ul>
              <p className="text-sm font-semibold">{t('backups.backupsTab.thisStaysAsIt')}</p>
              <ul className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">{preview.will_keep.map((l, i) => <li key={i}>{l}</li>)}</ul>
              {preview.warnings.map((w, i) => <Alert key={i} tone="warning">{w}</Alert>)}
            </div>
          )}
          {run.phase === 'running' && <JobBar events={run.events} label={t('backups.backupsTab.restoreProgress')} />}
          <ErrorNotice error={err ?? (run.phase === 'failed' ? run.error : null)} />
          {run.phase === 'done' && run.result && <Report r={run.result} />}
          <div className="flex justify-end gap-2">
            <Button disabled={run.phase === 'running'} onClick={onClose}>{run.phase === 'done' ? t('backups.backupsTab.close') : t('backups.backupsTab.cancel')}</Button>
            {ready && run.phase === 'idle' && <Button variant="destructive" disabled={!canRestore} title={why ?? undefined} onClick={() => void start()}>{t('backups.backupsTab.restoreNow')}</Button>}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function UndoBanner({ server, undo, onChanged, onDismiss, canRestore }: { server: string; undo: { id: string; created_unix: number; encrypted: boolean }; onChanged: () => void; onDismiss: () => void; canRestore: boolean }) {
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
  if (run.phase === 'done' && run.result) {
    return (
      <div className="space-y-3">
        <Report r={run.result} />
        <Button size="sm" onClick={onDismiss}>{t('backups.backupsTab.close')}</Button>
      </div>
    )
  }
  return (
    <Alert tone="info" title={t('backups.backupsTab.lastRestoreUndone')}>
      <p>{t('backups.backupsTab.justBeforeKoraSaved', { when: when(undo.created_unix) })}</p>
      {!open ? (
        <Button className="mt-2" size="sm" disabled={!canRestore} onClick={() => setOpen(true)}>{rich('backups.backupsTab.undoNodeUndoRestore', { undo2: <Undo2 /> })}</Button>
      ) : (
        <div className="mt-3 space-y-3">
          {undo.encrypted && <PasswordField value={pw} onChange={setPw} />}
          {run.phase === 'running' ? <JobBar events={run.events} label={t('backups.backupsTab.undoProgress')} /> : <Button variant="destructive" size="sm" disabled={undo.encrypted && !pw} onClick={() => void start()}>{t('backups.backupsTab.putServerBack')}</Button>}
          <ErrorNotice error={err ?? (run.phase === 'failed' ? run.error : null)} />
        </div>
      )}
    </Alert>
  )
}

// ---- the list --------------------------------------------------------------------------------------------------

const LABEL_TEXT: Record<BackupInfo['label'], Msg> = { manual: msg('backups.backupsTab.madeByYou'), scheduled: msg('backups.backupsTab.scheduled'), before_restore: msg('backups.backupsTab.safetyCopyBeforeRestore'), move: msg('backups.backupsTab.movingServer') }

function Row({ server, b, canAdmin, remote, onVerify, onRestore, onChanged }: { server: string; b: BackupInfo; canAdmin: boolean; remote: boolean; onVerify: () => void; onRestore: () => void; onChanged: () => void }) {
  const [confirm, setConfirm] = useState(false)
  const del = useMutation({ mutationFn: () => api.backups.remove(server, b.id), onSuccess: () => { setConfirm(false); onChanged() } })
  const dl = useMutation({ mutationFn: () => downloadFile(api.backups.fileUrl(server, b.id), b.file, b.bytes) })
  return (
    <li className="space-y-2 p-3 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{when(b.created_unix)}</span>
        <Badge tone="plain">{fileSize(b.bytes)}</Badge>
        {b.encrypted ? <Badge tone="success">{rich('backups.backupsTab.lockNodePasswordProtected', { lock: <Lock className="size-3" /> })}</Badge> : <Badge tone="neutral">{t('backups.backupsTab.passwordProtected')}</Badge>}
        <Badge tone="plain">{t(LABEL_TEXT[b.label])}</Badge>
        {b.stopped && <Badge tone="plain">{t('backups.backupsTab.serverStopped')}</Badge>}
        {b.too_new && <Badge tone="danger">{t('backups.backupsTab.madeNewerKora')}</Badge>}
        <span className="ml-auto flex flex-wrap gap-2">
          <Button size="sm" onClick={onVerify}>{rich('backups.backupsTab.shieldcheckNodeCheck', { shieldCheck: <ShieldCheck /> })}</Button>
          <Button size="sm" variant="outline" disabled={!canAdmin || b.too_new} onClick={onRestore}>{t('backups.backupsTab.restore')}</Button>
          {remote && <Button size="sm" disabled={!canAdmin || dl.isPending} onClick={() => dl.mutate()}>{rich('backups.backupsTab.downloadNodeDownload', { download: <Download /> })}</Button>}
          <Button size="sm" variant="outline" disabled={!canAdmin} onClick={() => setConfirm(true)} aria-label={t('backups.backupsTab.deleteBackupWhen', { when: when(b.created_unix) })}><Trash2 /></Button>
        </span>
      </div>
      <p className="text-xs text-muted-foreground">{t('backups.backupsTab.referenceResourcesReferenceFul', { by_reference: b.by_reference, in_full: b.in_full, value: b.database === 'dumped' ? 'included' : b.database === 'external' ? 'included (on another computer)' : b.database === 'none' ? 'none to back up' : 'not included', kora_version: b.kora_version })}</p>
      <ErrorNotice error={del.error ?? dl.error} />
      <ConfirmDialog open={confirm} onOpenChange={setConfirm} title={t('backups.backupsTab.deleteBackup')} description={t('backups.backupsTab.fileRemovedFromDisk')} confirmLabel={t('backups.backupsTab.delete')} destructive busy={del.isPending} onConfirm={() => del.mutate()} />
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
      <CardHeader><CardTitle>{t('backups.backupsTab.whereBackupsGoWhen')}</CardTitle></CardHeader>
      <CardContent className="space-y-5">
        {why && <Alert tone="info">{why}</Alert>}
        <div className="space-y-2">
          <div className="text-sm font-semibold">{t('backups.backupsTab.backupsKept')}</div>
          <CopyField value={view.folder} label={t('backups.backupsTab.backupsFolder')} />
          <p className="text-xs text-muted-foreground">{t('backups.backupsTab.valueBackupSurvivesDead', { value: view.default_folder ? t('backups.backupsTab.korasOwnBackupsFolder') : '', value2: view.free_bytes != null ? ` ${fileSize(view.free_bytes)} free.` : '' })}</p>
          <Field label={t('backups.backupsTab.useAnotherFolderFull')}><Input value={s.folder} disabled={!canChange} placeholder={t('backups.backupsTab.mntBackupsDBackups')} onChange={(e) => setS({ ...s, folder: e.target.value })} /></Field>
          {inShell() && <Button type="button" disabled={!canChange} onClick={async () => { const d = await browseForFolder(t('backups.backupsTab.chooseFolderBackups')); if (d) setS({ ...s, folder: d }) }}>{t('backups.backupsTab.browse')}</Button>}
        </div>
        <fieldset className="space-y-2">
          <legend className="mb-1 text-sm font-semibold">{t('backups.backupsTab.scheduledBackups')}</legend>
          <Radio name="when" checked={!sch.enabled} disabled={!canChange} onChange={() => setSch({ enabled: false })} label={t('backups.backupsTab.off')} hint={t('backups.backupsTab.backupsOnlyHappenWhen')} />
          <Radio name="when" checked={sch.enabled && sch.cadence === 'daily'} disabled={!canChange} onChange={() => setSch({ enabled: true, cadence: 'daily', weekday: undefined })} label={t('backups.backupsTab.everyDay')} />
          <Radio name="when" checked={sch.enabled && sch.cadence === 'weekly'} disabled={!canChange} onChange={() => setSch({ enabled: true, cadence: 'weekly', weekday: sch.weekday ?? 6 })} label={t('backups.backupsTab.onceWeek')} />
        </fieldset>
        {sch.enabled && (
          <div className="space-y-3">
            <div className="flex flex-wrap gap-4">
              {sch.cadence === 'weekly' && <div className="w-44"><Field label={t('backups.backupsTab.day')}><Select value={sch.weekday ?? 6} disabled={!canChange} onChange={(e) => setSch({ weekday: Number(e.target.value) })}>{DAYS.map((d, i) => <option key={d} value={i}>{d}</option>)}</Select></Field></div>}
              <div className="w-32"><Field label={t('backups.backupsTab.timeHours')} error={badTime ? t('backups.backupsTab.useFormHhMm') : null}><Input value={sch.time} disabled={!canChange} onChange={(e) => setSch({ time: e.target.value })} inputMode="numeric" placeholder="03:30" /></Field></div>
            </div>
            <Checkbox checked={s.stop_for_scheduled} disabled={!canChange} onChange={(e) => setS({ ...s, stop_for_scheduled: e.target.checked })} label={t('backups.backupsTab.stopServerDuringScheduled')} hint={t('backups.backupsTab.perfectlyConsistentCopyBut')} />
            <Alert tone="info">{view.schedule_note}</Alert>
          </div>
        )}
        <fieldset className="space-y-2">
          <legend className="mb-1 text-sm font-semibold">{t('backups.backupsTab.howManyKeep')}</legend>
          <div className="flex flex-wrap gap-4">
            <div className="w-44"><Field label={t('backups.backupsTab.newestBackupsAll')}><Input type="number" min={0} max={1000} value={s.retention.keep_last} disabled={!canChange} onChange={(e) => setS({ ...s, retention: { ...s.retention, keep_last: Math.max(0, Number(e.target.value) || 0) } })} /></Field></div>
            <div className="w-56"><Field label={t('backups.backupsTab.alsoOnePerDay')}><Input type="number" min={0} max={3650} value={s.retention.keep_daily_days} disabled={!canChange} onChange={(e) => setS({ ...s, retention: { ...s.retention, keep_daily_days: Math.max(0, Number(e.target.value) || 0) } })} /></Field></div>
          </div>
          <p className="text-xs text-muted-foreground">{t('backups.backupsTab.olderOnesRemovedAfter')}</p>
        </fieldset>
        <div className="flex flex-wrap gap-2">
          <Button variant="default" disabled={!canChange || save.isPending || badTime} title={why ?? undefined} onClick={() => save.mutate()}>{t('backups.backupsTab.save')}</Button>
          <Button disabled={!canChange || prune.isPending} onClick={() => prune.mutate()}>{t('backups.backupsTab.removeOldBackupsNow')}</Button>
        </div>
        {save.isSuccess && <Alert tone="success">{t('backups.backupsTab.saved')}</Alert>}
        {prune.data && <Alert tone="success">{prune.data.removed.length ? t('backups.backupsTab.removedCountOldBackup', { count: prune.data.removed.length }) : t('backups.backupsTab.nothingRemove')}</Alert>}
        <ErrorNotice error={save.error ?? prune.error} />
      </CardContent>
    </Card>
  )
}

// ---- move this server ------------------------------------------------------------------------------------------

/** "Move this server to another computer": a backup that travels, so a password is required. */
function MoveCard({ server: s, canMake, onDone }: { server: ServerDetail; canMake: boolean; onDone: () => void }) {
  const [pw, setPw] = useState('')
  const [pw2, setPw2] = useState('')
  const [job, setJob] = useState<string | null>(null)
  const [err, setErr] = useState<unknown>(null)
  const run = useJob<BackupCreated, BackupEvent>(job, () => onDone())
  const busy = run.phase === 'running'
  const problem = pw.length < MIN_PASSWORD ? t('backups.backupsTab.useLeastMinPassword', { MIN_PASSWORD }) : pw !== pw2 ? t('backups.backupsTab.twoPasswordsSame') : null
  const start = async () => {
    setErr(null)
    try {
      setJob((await api.backups.create(s.name, { passphrase: pw, everything: false, stop_server: false, for_move: true })).job)
    } catch (e) {
      setErr(e)
    }
  }
  return (
    <Card>
      <CardHeader><CardTitle>{t('backups.backupsTab.moveServerAnotherComputer')}</CardTitle></CardHeader>
      <CardContent className="space-y-4 text-sm">
        <ol className="list-decimal space-y-1 pl-5">
          <li>{t('backups.backupsTab.makeFileHereBelow')}</li>
          <li>{t('backups.backupsTab.copyFileNewComputer')}</li>
          <li>{t('backups.backupsTab.newComputerOpenKora')}</li>
          <li>{t('backups.backupsTab.whenServerRunsThere')}</li>
        </ol>
        <Alert tone="warning">{t('backups.backupsTab.licenceKeyMovesServer')}</Alert>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label={t('backups.backupsTab.passwordFile')} error={pw && problem ? problem : null}><Input type="password" autoComplete="new-password" value={pw} disabled={busy} onChange={(e) => setPw(e.target.value)} /></Field>
          <Field label={t('backups.backupsTab.passwordAgain')}><Input type="password" autoComplete="new-password" value={pw2} disabled={busy} onChange={(e) => setPw2(e.target.value)} /></Field>
        </div>
        <p className="text-xs text-muted-foreground">{t('backups.backupsTab.koraDoesKeepPassword2')}</p>
        <Button variant="default" disabled={!canMake || busy || !!problem} onClick={() => void start()}>{busy ? t('backups.backupsTab.makingFile') : t('backups.backupsTab.makeFileMove')}</Button>
        {busy && <JobBar events={run.events} label={t('backups.backupsTab.exportProgress')} />}
        <ErrorNotice error={err ?? (run.phase === 'failed' ? run.error : null)} />
        {run.phase === 'done' && run.result && (
          <Alert tone="success" title={t('backups.backupsTab.fileReady')}>
            <p>{run.result.info.file} · {fileSize(run.result.info.bytes)}</p>
            <p className="mt-1">{rich('backups.backupsTab.computerRunsKoraSpan', { span: <span className="font-mono text-xs">{run.result.path}</span> })}</p>
          </Alert>
        )}
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
  // The undo offer stays on screen after it has been used, so its result can be read (the list no longer has it).
  const [undo, setUndo] = useState<NonNullable<BackupsView['undo']> | null>(null)
  const changed = () => {
    void qc.invalidateQueries({ queryKey: ['backups', s.name] })
    void qc.invalidateQueries({ queryKey: ['backups-overview'] })
  }
  const v = view.data
  useEffect(() => {
    if (v?.undo) setUndo(v.undo)
  }, [v?.undo])
  if (noList) return <Alert tone="info">{noList}</Alert>
  return (
    <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
      <div className="space-y-5">
        <ErrorNotice error={view.error} />
        {v && (
          <Card>
            <CardHeader><CardTitle>{v.summary.text}</CardTitle></CardHeader>
            <CardContent className="space-y-3 text-sm">
              {v.summary.warning && <Alert tone="warning">{v.summary.warning}</Alert>}
              <p className="text-muted-foreground">{v.database.note}</p>
              {v.database.tool_missing && <Alert tone="warning" title={t('backups.backupsTab.databaseToolsMissing')}>{t('backups.backupsTab.koraNeedsMariadbdumpMysqldump')}</Alert>}
            </CardContent>
          </Card>
        )}
        {v && !v.settings.schedule.enabled && <BackupSuggestion server={s.name} />}
        {undo && <UndoBanner server={s.name} undo={undo} onChanged={changed} onDismiss={() => setUndo(null)} canRestore={!whyAdmin} />}
        <MakeBackup server={s} onDone={changed} />
        <Card>
          <CardHeader><CardTitle>{t('backups.backupsTab.backupsValue', { value: v ? ` (${v.backups.length})` : '' })}</CardTitle></CardHeader>
          <CardContent>
            {v && v.backups.length === 0 && <p className="text-sm text-muted-foreground">{t('backups.backupsTab.noBackupYetPress')}</p>}
            {v && v.backups.length > 0 && (
              <ul className="divide-y divide-border rounded-md border border-border">
                {v.backups.map((b) => <Row key={b.id} server={s.name} b={b} canAdmin={!whyAdmin} remote={role.remote} onVerify={() => setVerify(b)} onRestore={() => setRestore(b)} onChanged={changed} />)}
              </ul>
            )}
            {whyAdmin && v && v.backups.length > 0 && <p className="mt-2 text-xs text-muted-foreground">{whyAdmin}</p>}
          </CardContent>
        </Card>
      </div>
      <div className="space-y-5">
        {v && <Settings server={s.name} view={v} canChange={!whyAdmin} why={whyAdmin} onSaved={changed} />}
        <MoveCard server={s} canMake={!whyAdmin} onDone={changed} />
      </div>
      {verify && <VerifyDialog server={s.name} backup={verify} onClose={() => setVerify(null)} />}
      {restore && <RestoreDialog server={s} backup={restore} canRestore={!whyAdmin} why={whyAdmin} onClose={() => setRestore(null)} onChanged={changed} />}
    </div>
  )
}
