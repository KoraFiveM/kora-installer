import { Link } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { BackupSummary } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { timeAgo } from '@/lib/ago'
import { t } from '@/i18n'
import '@/i18n/en/core/backupChip'

/** The last backup of every server, one request for the whole list. A panel that cannot answer (an older one) leaves the line out. */
export function useBackupLines() {
  const q = useQuery({ queryKey: ['backups-overview'], queryFn: api.backups.overview, retry: false, staleTime: 30_000 })
  const by = new Map((q.data?.servers ?? []).map((l) => [l.server, l.summary]))
  return { by, ready: q.isSuccess }
}

/** "No backup yet" / "Last backup 3 days ago", amber when it is late or the last one failed. A link to the Backups tab. */
export function BackupChip({ server, summary }: { server: string; summary: BackupSummary | undefined }) {
  if (!summary) return null
  const to = `/servers/${encodeURIComponent(server)}?tab=backups`
  // The words are made here from the moment of the last backup (the server's own `text` is English).
  const when = summary.last_ok_unix ? t('core.backupChip.lastBackup', { ago: timeAgo(summary.last_ok_unix * 1000, Date.now()) }) : t('core.backupChip.noBackupYet')
  const tone = summary.warning ? 'amber' : summary.last_ok_unix ? 'plain' : 'neutral'
  return (
    <Link to={to} className="inline-block" title={summary.warning ?? undefined}>
      <Badge tone={tone}>{summary.warning ? t('core.backupChip.textNeedsLook', { text: when }) : when}</Badge>
    </Link>
  )
}
