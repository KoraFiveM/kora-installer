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

type Stage = 'files' | 'db' | 'name'

/** Staged like the TUI: keep files? keep database? then type the server name. Adopted servers only get "Remove from list". */
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
  const view = useJob(job, (j) => {
    if (j.state === 'done') {
      void qc.invalidateQueries({ queryKey: ['servers'] })
      push('success', `${s.name} was deleted.`)
      onDone?.()
    }
  })

  const reset = () => { setStage('files'); setTyped(''); setKeepFiles(true); setKeepDb(true); setErr(null) }
  const close = (o: boolean) => { setOpen(o); if (!o) reset() }

  if (!s.managed) {
    return (
      <Card className="max-w-2xl border-destructive/50">
        <CardHeader><CardTitle>Remove from list</CardTitle></CardHeader>
        <CardContent className="space-y-3">
          <p className="text-sm text-muted-foreground">You added this server from an existing folder. Kora will stop managing it, but your files and database are not touched.</p>
          <Button variant="destructive" disabled={blocked !== null} onClick={() => setOpen(true)}>Remove from list</Button>
          {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
          <ErrorNotice error={err} />
          <ConfirmDialog open={open} onOpenChange={close} title={`Remove ${s.name} from the list?`} description="Your files stay where they are." confirmLabel="Remove from list" destructive
            onConfirm={async () => { try { await api.servers.unregister(s.name); await qc.invalidateQueries({ queryKey: ['servers'] }); push('success', `${s.name} was removed from your list.`); onDone?.() } catch (e) { setErr(e); setOpen(false) } }} />
        </CardContent>
      </Card>
    )
  }

  const start = async () => {
    setErr(null)
    try {
      setJob((await api.servers.delete(s.name, { confirm_name: typed, keep_files: keepFiles, keep_db: keepDb })).job)
      setOpen(false)
    } catch (e) { setErr(e) }
  }

  return (
    <Card className="max-w-2xl border-destructive/50">
      <CardHeader><CardTitle>Delete server</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">Stops the server, removes its background service and firewall rules, and takes it out of your list. You choose what happens to the files and the database.</p>
        <Button variant="destructive" disabled={view.phase === 'running' || blocked !== null} onClick={() => setOpen(true)}>Delete server...</Button>
        {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
        {view.phase !== 'idle' && <ResourceProgress events={view.events} />}
        {view.phase === 'failed' && <ErrorNotice error={view.error} title="The server could not be deleted" />}
        <ErrorNotice error={err} />
        <ConfirmDialog open={open} onOpenChange={close}
          title={stage === 'files' ? 'Keep the server files?' : stage === 'db' ? 'Keep the database?' : `Delete ${s.name}`}
          description={stage === 'files' ? `The folder is ${s.dir}` : stage === 'db' ? 'The database holds your players, characters and everything they own.' : 'This cannot be undone.'}
          confirmLabel={stage === 'name' ? 'Delete server' : 'Next'} destructive={stage === 'name'}
          busy={stage === 'name' && typed !== s.name}
          onConfirm={() => { if (stage === 'files') setStage('db'); else if (stage === 'db') setStage('name'); else if (typed === s.name) void start() }}>
          {stage === 'files' && (
            <div className="space-y-2" role="radiogroup" aria-label="Server files">
              <label className="flex items-center gap-2 text-sm"><input type="radio" name="files" className="accent-[#a8ff00]" checked={keepFiles} onChange={() => setKeepFiles(true)} /> Keep the files</label>
              <label className="flex items-center gap-2 text-sm"><input type="radio" name="files" className="accent-[#a8ff00]" checked={!keepFiles} onChange={() => setKeepFiles(false)} /> Delete the files</label>
            </div>
          )}
          {stage === 'db' && (
            <div className="space-y-2" role="radiogroup" aria-label="Database">
              <label className="flex items-center gap-2 text-sm"><input type="radio" name="db" className="accent-[#a8ff00]" checked={keepDb} onChange={() => setKeepDb(true)} /> Keep the database</label>
              <label className="flex items-center gap-2 text-sm"><input type="radio" name="db" className="accent-[#a8ff00]" checked={!keepDb} onChange={() => setKeepDb(false)} /> Delete the database</label>
            </div>
          )}
          {stage === 'name' && (
            <div className="space-y-3">
              <Alert tone="danger">{keepFiles ? 'Files kept.' : 'The files will be deleted.'} {keepDb ? 'Database kept.' : 'The database will be deleted.'}</Alert>
              <Field label={`Type ${s.name} to confirm`}><Input value={typed} onChange={(e) => setTyped(e.target.value)} autoComplete="off" /></Field>
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
