import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ScanCandidate } from '@/api/types'
import { useAlerts } from '@/lib/app'
import { useJob } from '@/hooks/useJob'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Field } from '@/components/Field'
import { ErrorNotice } from '@/components/ErrorNotice'
import { ModeChip } from '@/components/Status'

export function AddExistingDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const qc = useQueryClient()
  const { push } = useAlerts()
  const [job, setJob] = useState<string | null>(null)
  const [dir, setDir] = useState('')
  const [error, setError] = useState<unknown>(null)
  const [busy, setBusy] = useState(false)
  const scan = useJob<{ candidates: ScanCandidate[] }>(job)

  const adopt = async (d: string) => {
    setBusy(true)
    setError(null)
    try {
      const s = await api.servers.adopt(d)
      await qc.invalidateQueries({ queryKey: ['servers'] })
      push('success', `${s.name} was added to your list.`)
      onOpenChange(false)
    } catch (e) {
      setError(e)
    } finally {
      setBusy(false)
    }
  }
  const start = async () => {
    setError(null)
    try {
      setJob((await api.servers.scan()).job)
    } catch (e) {
      setError(e)
    }
  }
  const candidates = scan.result?.candidates ?? []

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title="Add an existing server" description="Kora can manage a FiveM server you set up another way. It will not change your files.">
        <div className="space-y-5">
          <section className="space-y-2">
            <h3 className="text-sm font-semibold">Look for servers on this computer</h3>
            <Button onClick={start} disabled={scan.phase === 'running'}>{scan.phase === 'running' ? 'Looking...' : 'Scan for servers'}</Button>
            {scan.phase === 'done' && candidates.length === 0 && <p className="text-sm text-muted-foreground">No new servers were found. You can type the folder below.</p>}
            {scan.phase === 'failed' && <ErrorNotice error={scan.error} />}
            <ul className="space-y-2">
              {candidates.map((c) => (
                <li key={c.dir} className="flex items-center justify-between gap-3 rounded-md border border-border p-3 text-sm">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2 font-medium">{c.name} <ModeChip mode={c.mode} /></div>
                    <div className="truncate font-mono text-xs text-muted-foreground">{c.dir}</div>
                  </div>
                  <Button size="sm" disabled={busy} onClick={() => adopt(c.dir)} aria-label={`Add ${c.name}`}>Add</Button>
                </li>
              ))}
            </ul>
          </section>
          <form
            className="space-y-2"
            onSubmit={(e) => {
              e.preventDefault()
              if (dir.trim()) void adopt(dir.trim())
            }}
          >
            <h3 className="text-sm font-semibold">Or type the folder</h3>
            <Field label="Server folder" hint="The folder that contains server.cfg and resources, for example /srv/fivem/my-server.">
              <Input value={dir} onChange={(e) => setDir(e.target.value)} placeholder="/srv/fivem/my-server" />
            </Field>
            <Button type="submit" variant="default" disabled={busy || !dir.trim()}>Add this folder</Button>
          </form>
          <ErrorNotice error={error} />
        </div>
      </DialogContent>
    </Dialog>
  )
}
