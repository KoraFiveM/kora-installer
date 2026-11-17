import { useState } from 'react'
import { Link } from 'react-router-dom'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { BackupEvent, ImportChoices, ImportPreview, ImportResult, ImportSource } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useRole } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ErrorNotice } from '@/components/ErrorNotice'
import { DoneStep, FileStep, PasswordStep, PreviewStep, RunStep } from './steps'

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
  const [start, setStart] = useState(true)
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
      setJob((await api.imports.start({ source, passphrase: password, choices, licence_key_name: keyName ?? undefined, start, start_over: startOver })).job)
    } catch (e) {
      setStartErr(e)
    }
  }

  if (!role.can('admin')) {
    return (
      <div className="mx-auto max-w-2xl space-y-5">
        <h1 className="font-heading text-2xl font-bold">Bring a server from a backup or another machine</h1>
        <Alert tone="info">Your account ({role.role}) cannot bring a server in. Ask an administrator.</Alert>
        <Button asChild><Link to="/">Back to your servers</Link></Button>
      </div>
    )
  }
  const stage = run.phase === 'done' ? 'done' : job ? 'run' : preview && !preview.needs_passphrase && choices ? 'preview' : preview?.needs_passphrase ? 'password' : 'file'
  return (
    <div className="mx-auto max-w-2xl space-y-5">
      <h1 className="font-heading text-2xl font-bold">Bring a server from a backup or another machine</h1>
      <p className="text-sm text-muted-foreground">Restore a server from a backup file, or move one here from another computer (Linux or Windows). On the old computer open the server's Backups tab and choose &quot;Move this server&quot; to make the file. Nothing leaves your computers except the file you carry yourself.</p>
      <Card>
        <CardHeader><CardTitle>{stage === 'file' ? '1. The file' : stage === 'password' ? '2. The password' : stage === 'preview' ? '3. What will happen' : stage === 'run' ? '4. Bringing it in' : 'Done'}</CardTitle></CardHeader>
        <CardContent>
          {stage === 'file' && <FileStep local={!role.remote} busy={upload.isPending || look.isPending} error={upload.error ?? look.error} onFile={(f) => upload.mutate(f)} onPath={(path) => { setFileName(path); look.mutate({ src: { path } }) }} />}
          {stage === 'password' && <PasswordStep name={fileName || preview?.file_name || 'The file'} error={look.error} busy={look.isPending} onPassword={(pw) => source && look.mutate({ src: source, pw })} />}
          {stage === 'preview' && preview && choices && <PreviewStep preview={preview} choices={choices} setChoices={setChoices} key_={keyName} setKey={setKeyName} start={start} setStart={setStart} busy={false} onGo={() => void go(false)} />}
          {stage === 'preview' && startErr != null && <div className="mt-3"><ErrorNotice error={startErr} /></div>}
          {stage === 'run' && <RunStep events={run.events} failed={run.phase === 'failed' ? run.error : null} onRetry={() => void go(false)} onStartOver={() => void go(true)} />}
          {stage === 'done' && run.result && <DoneStep r={run.result} />}
        </CardContent>
      </Card>
    </div>
  )
}
