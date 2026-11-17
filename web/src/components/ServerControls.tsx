import { useState } from 'react'
import { Play, RotateCw, Square } from 'lucide-react'
import type { ServerStatus } from '@/api/types'
import type { ControlAction } from '@/api/types'
import { Button } from '@/components/ui/button'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { useRole, whyNot } from '@/hooks/useRole'
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/servers'

export function ServerControls({ name, status, busy, run, size = 'sm' }: { name: string; status: ServerStatus; busy: boolean; run: (a: ControlAction) => void; size?: 'sm' | 'default' }) {
  const [confirm, setConfirm] = useState(false)
  const role = useRole()
  const off = whyNot(role, 'operator')
  if (status === 'missing') return <span className="text-xs text-muted-foreground">{t('servers.serverControls.folderMissing')}</span>
  return (
    <div className="flex gap-2" title={off ?? undefined}>
      {status === 'stopped' && (
        <Button size={size} disabled={busy || off !== null} onClick={() => run('start')} aria-label={t('servers.serverControls.startName', { name })}>{rich('servers.serverControls.playNodeStart', { play: <Play /> })}</Button>
      )}
      {status === 'running' && (
        <>
          <Button size={size} disabled={busy || off !== null} onClick={() => run('stop')} aria-label={t('servers.serverControls.stopName', { name })}>{rich('servers.serverControls.squareNodeStop', { square: <Square /> })}</Button>
          <Button size={size} disabled={busy || off !== null} onClick={() => setConfirm(true)} aria-label={t('servers.serverControls.restartName', { name })}>{rich('servers.serverControls.rotatecwNodeRestart', { rotateCw: <RotateCw /> })}</Button>
        </>
      )}
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title={t('servers.serverControls.restartName2', { name })}
        description={t('servers.serverControls.everyonePlayingServerDisconnec')}
        confirmLabel={t('servers.serverControls.restartServer')}
        onConfirm={() => {
          setConfirm(false)
          run('restart')
        }}
      />
    </div>
  )
}
