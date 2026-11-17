import { useState } from 'react'
import { Link } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ArchivedServer, BackupEvent, DbAdmin, ForkChoice, ImportChoices, ImportPreview, ImportResult, ImportSource, RunMode } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useRole } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ErrorNotice } from '@/components/ErrorNotice'
import { DoneStep, FileStep, PasswordStep, PreviewStep, RunStep } from './steps'
import { t } from '@/i18n'
import '@/i18n/en/backups/importPage'

/** "Bring a server from a backup or another machine": its own page and steps, next to (not inside) the install wizard. */
export default function ImportPage() {
  const role = useRole()
  const qc = useQueryClient()
  const [source, setSource] = useState<ImportSource | null>(null)
  const [fileName, setFileName] = useState('')
  const [password, setPassword] = useState<string | undefined>(undefined)
  const [preview, setPreview] = useState<ImportPreview | null>(null)
  const [choices, setChoices] = useState<ImportChoices | null>(null)
  const [keyName, setKeyName] = useState<string | null>(null)
  const [forkChoice, setForkChoice] = useState<ForkChoice>('keep')
  const [admin, setAdmin] = useState<DbAdmin | null>(null)
  const [start, setStart] = useState(true)
  /** How the new server runs here: as the backed-up one did when this computer can do the same. `null` = not asked (a service). */
  const [runMode, setRunMode] = useState<RunMode | null>(null)
  const archived = useQuery({ queryKey: ['backups-archived'], queryFn: api.backups.archived, retry: false })
  const [job, setJob] = useState<string | null>(null)
  const [startErr, setStartErr] = useState<unknown>(null)
  const run = useJob<ImportResult, BackupEvent>(job, () => void qc.invalidateQueries({ queryKey: ['servers'] }))

  const look = useMutation({
    mutationFn: ({ src, pw }: { src: ImportSource; pw?: string }) => api.imports.inspect(src, pw),
    onSuccess: (p, v) => {
      setSource(v.src)
      setPassword(v.pw)
      setPreview(p)
      if (p.suggested) setChoices(p.suggested)
      setRunMode(p.run_mode === 'container' && p.container_engine ? 'container' : p.container_engine ? 'service' : null)
    },
  })
  const upload = useMutation({
    mutationFn: async (f: File) => {
      const up = await api.imports.upload(f, f.name)
      setFileName(up.file_name)
      return { src: { upload: up.upload } as ImportSource }
    },
    onSuccess: ({ src }) => look.mutate({ src }),
  })

  const go = async (startOver: boolean) => {
    if (!source || !choices) return
    setStartErr(null)
    try {
      setJob((await api.imports.start({ source, passphrase: password, choices, licence_key_name: keyName ?? undefined, fork_choice: forkChoice, db_admin: admin ?? undefined, start, start_over: startOver, ...(runMode ? { run_mode: runMode } : {}) })).job)
    } catch (e) {
      setStartErr(e)
    }
  }

  if (!role.can('admin')) {
    return (
      <div className="mx-auto max-w-2xl space-y-5">
        <h1 className="font-heading text-2xl font-bold">{t('backups.importPage.bringServerFromBackup')}</h1>
        <Alert tone="info">{t('backups.importPage.accountRoleCannotBring', { role: role.role })}</Alert>
        <Button asChild><Link to="/">{t('backups.importPage.backServers')}</Link></Button>
      </div>
    )
  }
  const stage = run.phase === 'done' ? 'done' : job ? 'run' : preview && !preview.needs_passphrase && choices ? 'preview' : preview?.needs_passphrase ? 'password' : 'file'
  return (
    <div className="mx-auto max-w-2xl space-y-5">
      <h1 className="font-heading text-2xl font-bold">{t('backups.importPage.bringServerFromBackup')}</h1>
      <p className="text-sm text-muted-foreground">{t('backups.importPage.restoreServerFromBackup')}</p>
      <Card>
        <CardHeader><CardTitle>{stage === 'file' ? t('backups.importPage.theFile') : stage === 'password' ? t('backups.importPage.thePassword') : stage === 'preview' ? t('backups.importPage.whatHappen') : stage === 'run' ? t('backups.importPage.bringingItIn') : t('backups.importPage.done')}</CardTitle></CardHeader>
        <CardContent>
          {stage === 'file' && <FileStep local={!role.remote} busy={upload.isPending || look.isPending} error={upload.error ?? look.error} archived={archived.data?.servers ?? []} onArchived={(a: ArchivedServer, id) => { setFileName(`${a.name} (deleted)`); look.mutate({ src: { archived: { name: a.name, deleted_unix: a.deleted_unix, backup: id } } }) }} onFile={(f) => upload.mutate(f)} onPath={(path) => { setFileName(path); look.mutate({ src: { path } }) }} />}
          {stage === 'password' && <PasswordStep name={fileName || preview?.file_name || 'The file'} error={look.error} busy={look.isPending} onPassword={(pw) => source && look.mutate({ src: source, pw })} />}
          {stage === 'preview' && preview && choices && <PreviewStep preview={preview} choices={choices} setChoices={setChoices} key_={keyName} setKey={setKeyName} forkChoice={forkChoice} setForkChoice={setForkChoice} admin={admin} setAdmin={setAdmin} start={start} setStart={setStart} runMode={runMode} setRunMode={setRunMode} busy={false} onGo={() => void go(false)} />}
          {stage === 'preview' && startErr != null && <div className="mt-3"><ErrorNotice error={startErr} /></div>}
          {stage === 'run' && <RunStep events={run.events} failed={run.phase === 'failed' ? run.error : null} onRetry={() => void go(false)} onStartOver={() => void go(true)} />}
          {stage === 'done' && run.result && <DoneStep r={run.result} />}
        </CardContent>
      </Card>
    </div>
  )
}
