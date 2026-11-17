import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ServerDetail } from '@/api/types'
import { useAlerts } from '@/lib/app'
import { useJob } from '@/hooks/useJob'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { ResourceProgress } from '@/components/JobProgress'
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/servers/dangerTab'

type Stage = 'files' | 'db' | 'name'

/** Staged: keep files? keep database? then type the server name. Adopted servers only get "Remove from list". */
export function DeleteFlow({ server: s, onDone }: { server: Pick<ServerDetail, 'name' | 'managed' | 'dir' | 'status'>; onDone?: () => void }) {
  const qc = useQueryClient()
  const { push } = useAlerts()
  const role = useRole()
  const blocked = whyNot(role, 'admin', true)
  const [open, setOpen] = useState(false)
  const [stage, setStage] = useState<Stage>('files')
  const [keepFiles, setKeepFiles] = useState(true)
  const [keepDb, setKeepDb] = useState(true)
  const [typed, setTyped] = useState('')
  const [job, setJob] = useState<string | null>(null)
  const [err, setErr] = useState<unknown>(null)
  const [dbPassword, setDbPassword] = useState('')
  const view = useJob(job, (j) => {
    if (j.state === 'done') {
      void qc.invalidateQueries({ queryKey: ['servers'] })
      push('success', t('servers.dangerTab.deleted', { name: s.name }))
      onDone?.()
    }
  })

  const reset = () => { setStage('files'); setTyped(''); setKeepFiles(true); setKeepDb(true); setErr(null) }
  const close = (o: boolean) => { setOpen(o); if (!o) reset() }

  if (!s.managed) {
    return (
      <Card className="max-w-2xl border-destructive/50">
        <CardHeader><CardTitle>{t('servers.dangerTab.removeFromList')}</CardTitle></CardHeader>
        <CardContent className="space-y-3">
          <p className="text-sm text-muted-foreground">{t('servers.dangerTab.addedServerFromExisting')}</p>
          <Button variant="destructive" disabled={blocked !== null} onClick={() => setOpen(true)}>{t('servers.dangerTab.removeFromList')}</Button>
          {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
          <ErrorNotice error={err} />
          <ConfirmDialog open={open} onOpenChange={close} title={t('servers.dangerTab.removeNameFromList', { name: s.name })} description={t('servers.dangerTab.filesStayWhereThey')} confirmLabel={t('servers.dangerTab.removeFromList')} destructive
            onConfirm={async () => { try { await api.servers.unregister(s.name); await qc.invalidateQueries({ queryKey: ['servers'] }); push('success', t('servers.dangerTab.removedFromList', { name: s.name })); onDone?.() } catch (e) { setErr(e); setOpen(false) } }} />
        </CardContent>
      </Card>
    )
  }

  const start = async (opts: { keepDb?: boolean; password?: string } = {}) => {
    setErr(null)
    try {
      setJob((await api.servers.delete(s.name, { confirm_name: typed, keep_files: keepFiles, keep_db: opts.keepDb ?? keepDb, ...(opts.password ? { db_admin_password: opts.password } : {}) })).job)
      setOpen(false)
    } catch (e) { setErr(e) }
  }
  const needsPassword = view.phase === 'failed' && (view.error as { code?: string } | null)?.code === 'needs_db_password'

  return (
    <Card className="max-w-2xl border-destructive/50">
      <CardHeader><CardTitle>{t('servers.dangerTab.deleteServer')}</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">{t('servers.dangerTab.stopsServerRemovesIts')}</p>
        <Button variant="destructive" disabled={view.phase === 'running' || blocked !== null} onClick={() => setOpen(true)}>{t('servers.dangerTab.deleteServer2')}</Button>
        {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
        {view.phase !== 'idle' && <ResourceProgress events={view.events} />}
        {view.phase === 'failed' && <ErrorNotice error={view.error} title={t('servers.dangerTab.serverCouldDeleted')} />}
        {needsPassword && (
          <div className="space-y-3 border border-border p-4" role="group" aria-label={t('servers.dangerTab.databaseLoginNeeded')}>
            <Field label={t('servers.dangerTab.databaseAdministratorPassword')} hint={t('servers.dangerTab.usedOnceDeleteDatabase')}>
              <Input type="password" value={dbPassword} onChange={(e) => setDbPassword(e.target.value)} autoComplete="off" />
            </Field>
            <div className="flex flex-wrap gap-2">
              <Button variant="destructive" disabled={dbPassword === '' || view.phase !== 'failed'} onClick={() => { const p = dbPassword; setDbPassword(''); void start({ password: p }) }}>{t('servers.dangerTab.tryAgainPassword')}</Button>
              <Button variant="outline" onClick={() => { setKeepDb(true); setDbPassword(''); void start({ keepDb: true }) }}>{t('servers.dangerTab.keepDatabaseFinish')}</Button>
            </div>
          </div>
        )}
        <ErrorNotice error={err} />
        <ConfirmDialog open={open} onOpenChange={close}
          title={stage === 'files' ? t('servers.dangerTab.keepServerFiles') : stage === 'db' ? t('servers.dangerTab.keepDatabase') : t('servers.dangerTab.deleteName', { name: s.name })}
          description={stage === 'files' ? t('servers.dangerTab.folderDir', { dir: s.dir }) : stage === 'db' ? t('servers.dangerTab.databaseHoldsPlayersCharacters') : t('servers.dangerTab.cannotUndone')}
          confirmLabel={stage === 'name' ? t('servers.dangerTab.deleteServer') : t('servers.dangerTab.next')} destructive={stage === 'name'}
          busy={stage === 'name' && typed !== s.name}
          onConfirm={() => { if (stage === 'files') setStage('db'); else if (stage === 'db') setStage('name'); else if (typed === s.name) void start() }}>
          {stage === 'files' && (
            <div className="space-y-2" role="radiogroup" aria-label={t('servers.dangerTab.serverFiles')}>
              <label className="flex items-center gap-2 text-sm">{rich('servers.dangerTab.inputNodeKeepFiles', { input: <input type="radio" name="files" className="accent-lime" checked={keepFiles} onChange={() => setKeepFiles(true)} /> })}</label>
              <label className="flex items-center gap-2 text-sm">{rich('servers.dangerTab.inputNodeDeleteFiles', { input: <input type="radio" name="files" className="accent-lime" checked={!keepFiles} onChange={() => setKeepFiles(false)} /> })}</label>
            </div>
          )}
          {stage === 'db' && (
            <div className="space-y-2" role="radiogroup" aria-label={t('servers.dangerTab.database')}>
              <label className="flex items-center gap-2 text-sm">{rich('servers.dangerTab.inputNodeKeepDatabase', { input: <input type="radio" name="db" className="accent-lime" checked={keepDb} onChange={() => setKeepDb(true)} /> })}</label>
              <label className="flex items-center gap-2 text-sm">{rich('servers.dangerTab.inputNodeDeleteDatabase', { input: <input type="radio" name="db" className="accent-lime" checked={!keepDb} onChange={() => setKeepDb(false)} /> })}</label>
            </div>
          )}
          {stage === 'name' && (
            <div className="space-y-3">
              <Alert tone="danger">{keepFiles ? t('servers.dangerTab.filesKept') : t('servers.dangerTab.filesDeleted')} {keepDb ? t('servers.dangerTab.databaseKept') : t('servers.dangerTab.databaseDeleted')}</Alert>
              <Field label={t('servers.dangerTab.typeNameConfirm', { name: s.name })}><Input value={typed} onChange={(e) => setTyped(e.target.value)} autoComplete="off" /></Field>
            </div>
          )}
        </ConfirmDialog>
      </CardContent>
    </Card>
  )
}

export function DangerTab({ server: s }: { server: ServerDetail }) {
  const nav = useNavigate()
  return <DeleteFlow server={s} onDone={() => nav('/')} />
}
