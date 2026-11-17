import { Link } from 'react-router-dom'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { X } from 'lucide-react'
import { api } from '@/api/client'
import { usePanelAlerts } from '@/hooks/usePanelAlerts'
import { useRole } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { ErrorNotice } from '@/components/ErrorNotice'

/** Monitoring alerts at the top of every page, until someone acknowledges them. */
export function AlertsBanner() {
  const { open } = usePanelAlerts()
  const qc = useQueryClient()
  const canAck = useRole().can('operator')
  const ack = useMutation({ mutationFn: (id: number) => api.alerts.ack(id), onSuccess: () => qc.invalidateQueries({ queryKey: ['panel-alerts'] }) })
  if (open.length === 0) return null
  return (
    <div className="space-y-3" aria-label="Server alerts">
      {open.slice(0, 5).map((a) => (
        <div key={a.id} className="relative">
          <Alert tone={a.tone} title={<>{a.title}: <Link className="underline" to={`/servers/${encodeURIComponent(a.server)}?tab=${a.kind === 'update_attention' ? 'resources' : a.kind === 'backup_failed' ? 'backups' : 'monitoring'}`}>{a.server}</Link></>}>
            {a.message}
          </Alert>
          {canAck && (
            <button className="absolute right-3 top-3 text-muted-foreground hover:text-foreground" aria-label={`Dismiss the alert for ${a.server}`} title="Dismiss" onClick={() => ack.mutate(a.id)}>
              <X className="size-4" />
            </button>
          )}
        </div>
      ))}
      <ErrorNotice error={ack.error} title="The alert could not be dismissed" />
      {open.length > 5 && <p className="text-sm text-muted-foreground">And {open.length - 5} more alerts.</p>}
    </div>
  )
}
