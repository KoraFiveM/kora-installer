import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ScanCandidate } from '@/api/types'
import { useAlerts } from '@/lib/app'
import { browseForFolder, inShell } from '@/lib/shell'
import { useRole, whyNot } from '@/hooks/useRole'
import { useJob } from '@/hooks/useJob'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Field } from '@/components/Field'
import { ErrorNotice } from '@/components/ErrorNotice'
import { ModeChip } from '@/components/Status'
import { t } from '@/i18n'
import '@/i18n/en/servers/addExistingDialog'

export function AddExistingDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const qc = useQueryClient()
  const { push } = useAlerts()
  const [job, setJob] = useState<string | null>(null)
  const [dir, setDir] = useState('')
  const [error, setError] = useState<unknown>(null)
  const [busy, setBusy] = useState(false)
  const role = useRole()
  const noScan = whyNot(role, 'operator')
  const noAdd = whyNot(role, 'admin')
  const scan = useJob<{ candidates: ScanCandidate[] }>(job)

  const adopt = async (d: string) => {
    setBusy(true)
    setError(null)
    try {
      const s = await api.servers.adopt(d)
      await qc.invalidateQueries({ queryKey: ['servers'] })
      push('success', t('servers.addExistingDialog.addedToList', { name: s.name }))
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
      <DialogContent title={t('servers.addExistingDialog.addExistingServer')} description={t('servers.addExistingDialog.koraManageFivemServer')}>
        <div className="space-y-5">
          <section className="space-y-2">
            <h3 className="text-sm font-semibold">{t('servers.addExistingDialog.lookServersComputer')}</h3>
            <Button onClick={start} disabled={scan.phase === 'running' || !!noScan} title={noScan ?? undefined}>{scan.phase === 'running' ? t('servers.addExistingDialog.looking') : t('servers.addExistingDialog.scanServers')}</Button>
            {scan.phase === 'done' && candidates.length === 0 && <p className="text-sm text-muted-foreground">{t('servers.addExistingDialog.noNewServersWere')}</p>}
            {scan.phase === 'failed' && <ErrorNotice error={scan.error} />}
            <ul className="space-y-2">
              {candidates.map((c) => (
                <li key={c.dir} className="flex items-center justify-between gap-3 rounded-md border border-border p-3 text-sm">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2 font-medium">{c.name} <ModeChip mode={c.mode} /></div>
                    <div className="truncate font-mono text-xs text-muted-foreground">{c.dir}</div>
                  </div>
                  <Button size="sm" disabled={busy || !!noAdd} title={noAdd ?? undefined} onClick={() => adopt(c.dir)} aria-label={t('servers.addExistingDialog.addName', { name: c.name })}>{t('servers.addExistingDialog.add')}</Button>
                </li>
              ))}
            </ul>
          </section>
          <form
            className="space-y-2"
            onSubmit={(e) => {
              e.preventDefault()
              if (dir.trim() && !noAdd) void adopt(dir.trim())
            }}
          >
            <h3 className="text-sm font-semibold">{t('servers.addExistingDialog.typeFolder')}</h3>
            <Field label={t('servers.addExistingDialog.serverFolder')} hint={t('servers.addExistingDialog.folderContainsServerCfg')}>
              <Input value={dir} disabled={!!noAdd} onChange={(e) => setDir(e.target.value)} placeholder={t('servers.addExistingDialog.srvFivemMyserver')} />
            </Field>
            {inShell() && (
              <Button type="button" disabled={!!noAdd} onClick={async () => { const d = await browseForFolder(t('servers.addExistingDialog.chooseServerFolder')); if (d) setDir(d) }}>{t('servers.addExistingDialog.browse')}</Button>
            )}
            <Button type="submit" variant="default" disabled={busy || !dir.trim() || !!noAdd} title={noAdd ?? undefined}>{t('servers.addExistingDialog.addFolder')}</Button>
            {noAdd && <p className="text-xs text-muted-foreground">{noAdd}</p>}
          </form>
          <ErrorNotice error={error} />
        </div>
      </DialogContent>
    </Dialog>
  )
}
