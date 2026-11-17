import { useState } from 'react'
import { Link } from 'react-router-dom'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import { useRole } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { ErrorNotice } from '@/components/ErrorNotice'

/**
 * "Back this server up every day?": offered when a server is new (the install's last page) and on the Backups tab until
 * there is a schedule. It is a suggestion: nothing is switched on until the person presses the button, and it
 * changes only the schedule (every day at 03:30, the newest 7 kept).
 */
export function BackupSuggestion({ server, compact }: { server: string; compact?: boolean }) {
  const role = useRole()
  const qc = useQueryClient()
  const [done, setDone] = useState(false)
  const on = useMutation({
    mutationFn: async () => {
      const v = await api.backups.list(server)
      return api.backups.setSettings(server, { ...v.settings, schedule: { enabled: true, cadence: 'daily', time: '03:30' } })
    },
    onSuccess: () => {
      setDone(true)
      void qc.invalidateQueries({ queryKey: ['backups', server] })
      void qc.invalidateQueries({ queryKey: ['backups-overview'] })
    },
  })
  if (done) return <Alert tone="success" title="Daily backups are on">A backup is made every day at 03:30 while Kora&apos;s background service runs, and the newest 7 are kept. Change this on the <Link className="underline" to={`/servers/${encodeURIComponent(server)}?tab=backups`}>Backups tab</Link>.</Alert>
  return (
    <Alert tone="info" title="Back this server up every day?">
      <p>A disk that fails or an update that goes wrong can take your players&apos; data with it. Kora can make a backup of the database, settings and resources every day.{compact ? '' : ' It is off until you turn it on, and you can change when, where to, and how many are kept.'}</p>
      <div className="mt-2 flex flex-wrap gap-2">
        {role.can('admin') && <Button size="sm" variant="default" disabled={on.isPending} onClick={() => on.mutate()}>Turn on daily backups</Button>}
        <Button asChild size="sm"><Link to={`/servers/${encodeURIComponent(server)}?tab=backups`}>Backup settings</Link></Button>
      </div>
      <ErrorNotice error={on.error} />
    </Alert>
  )
}
