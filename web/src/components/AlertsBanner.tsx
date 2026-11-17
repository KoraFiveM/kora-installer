import { Link } from 'react-router-dom'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { X } from 'lucide-react'
import { api } from '@/api/client'
import { usePanelAlerts } from '@/hooks/usePanelAlerts'
import { useRole } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { ErrorNotice } from '@/components/ErrorNotice'
import { t } from '@/i18n'
import '@/i18n/en/core/alertsBanner'

/** Monitoring alerts at the top of every page, until someone acknowledges them. An alert with no server is about Kora itself (the panel's certificate, the administrator service): it opens Settings. */
export function AlertsBanner() {
  const { open } = usePanelAlerts()
  const qc = useQueryClient()
  const canAck = useRole().can('operator')
  const ack = useMutation({ mutationFn: (id: number) => api.alerts.ack(id), onSuccess: () => qc.invalidateQueries({ queryKey: ['panel-alerts'] }) })
  if (open.length === 0) return null
  return (
    <div className="space-y-3" aria-label={t('core.alertsBanner.serverAlerts')}>
      {open.slice(0, 5).map((a) => (
        <div key={a.id} className="relative">
          <Alert data-i18n-skip="server text: alert titles and messages come from the server in English" tone={a.tone} title={a.server ? <>{a.title}: <Link className="underline" to={`/servers/${encodeURIComponent(a.server)}?tab=${a.kind === 'update_attention' ? 'resources' : a.kind === 'backup_failed' ? 'backups' : 'monitoring'}`}>{a.server}</Link></> : <>{a.title}: <Link className="underline" to="/settings">{t('core.alertsBanner.whatDo')}</Link></>}>
            {a.message}
          </Alert>
          {canAck && (
            <button className="absolute right-2 top-2 rounded-md p-1.5 text-muted-foreground hover:text-foreground" aria-label={a.server ? t('core.alertsBanner.dismissAlertServer', { server: a.server }) : t('core.alertsBanner.dismissAlertTitle', { title: a.title })} title={t('core.alertsBanner.dismiss')} onClick={() => ack.mutate(a.id)}>
              <X className="size-4" />
            </button>
          )}
        </div>
      ))}
      <ErrorNotice error={ack.error} title={t('core.alertsBanner.alertCouldDismissed')} />
      {open.length > 5 && <p className="text-sm text-muted-foreground">{t('core.alertsBanner.valueMoreAlerts', { count: open.length - 5 })}</p>}
    </div>
  )
}
