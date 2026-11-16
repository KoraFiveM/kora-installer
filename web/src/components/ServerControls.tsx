import { useState } from 'react'
import { Play, RotateCw, Square } from 'lucide-react'
import type { ServerStatus } from '@/api/types'
import type { ControlAction } from '@/api/types'
import { Button } from '@/components/ui/button'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { useRole, whyNot } from '@/hooks/useRole'

export function ServerControls({ name, status, busy, run, size = 'sm' }: { name: string; status: ServerStatus; busy: boolean; run: (a: ControlAction) => void; size?: 'sm' | 'default' }) {
  const [confirm, setConfirm] = useState(false)
  const role = useRole()
  const off = whyNot(role, 'operator')
  if (status === 'missing') return <span className="text-xs text-muted-foreground">Folder missing</span>
  return (
    <div className="flex gap-2" title={off ?? undefined}>
      {status === 'stopped' && (
        <Button size={size} disabled={busy || off !== null} onClick={() => run('start')} aria-label={`Start ${name}`}>
          <Play /> Start
        </Button>
      )}
      {status === 'running' && (
        <>
          <Button size={size} disabled={busy || off !== null} onClick={() => run('stop')} aria-label={`Stop ${name}`}>
            <Square /> Stop
          </Button>
          <Button size={size} disabled={busy || off !== null} onClick={() => setConfirm(true)} aria-label={`Restart ${name}`}>
            <RotateCw /> Restart
          </Button>
        </>
      )}
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title={`Restart ${name}?`}
        description="Everyone playing on this server will be disconnected for a minute or so while it restarts."
        confirmLabel="Restart server"
        onConfirm={() => {
          setConfirm(false)
          run('restart')
        }}
      />
    </div>
  )
}
