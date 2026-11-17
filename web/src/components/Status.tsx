import { Link } from 'react-router-dom'
import type { Badge as BadgeT, Mode, ServerStatus, Source, Versions } from '@/api/types'
import { styleOfMode } from '@/pages/install/form'
import { Badge } from '@/components/ui/badge'
import { t } from '@/i18n'
import '@/i18n/en/core'

export function StatusChip({ status, pending }: { status: ServerStatus; pending?: string }) {
  if (pending) return <Badge tone="amber" dot>{pending}</Badge>
  if (status === 'running') return <Badge tone="success" dot>{t('core.status.running')}</Badge>
  if (status === 'stopped') return <Badge tone="danger" dot>{t('core.status.stopped')}</Badge>
  return <Badge tone="neutral" dot>{t('core.status.folderMissing')}</Badge>
}

function updateBadge(badge: BadgeT | null) {
  if (!badge) return <Badge tone="plain">{t('core.status.notChecked')}</Badge>
  switch (badge.kind) {
    case 'up_to_date':
      return <Badge tone="plain">{t('core.status.upDate')}</Badge>
    case 'updates':
      return <Badge tone="info">{badge.count === 1 ? t('core.status.update') : t('core.status.valueUpdates', { value: badge.count ?? 0 })}</Badge>
    case 'needs_attention':
      return <Badge tone="orange">{t('core.status.needsAttention')}</Badge>
    case 'new_release':
      return <Badge tone="info">{t('core.status.newRelease')}</Badge>
  }
}

/** `to`: when there is something to look at, the badge links there (the server's Resources tab). */
export function UpdateChip({ badge, to }: { badge: BadgeT | null; to?: string }) {
  const chip = updateBadge(badge)
  return to && badge && badge.kind !== 'up_to_date' ? <Link to={to} className="hover:underline">{chip}</Link> : chip
}

/** Where a server's resources come from and which versions it follows, in plain words ("Plain copies, released versions"). */
export function StyleChip({ source, versions }: { source: Source; versions: Versions }) {
  return (
    <Badge tone="plain" title={`${source === 'clone' ? t('core.status.plainCopiesOriginalRepositorie') : t('core.status.ownCopiesForksGithub')}; ${versions === 'released' ? 'pinned to released versions' : 'following the latest changes'}.`}>
      {source === 'clone' ? t('core.status.plainCopies') : t('core.status.ownCopies')}, {versions === 'released' ? t('core.status.releasedVersions') : t('core.status.latestChanges')}
    </Badge>
  )
}

/** The same chip from the old `mode` word (answers from the folder scan, which only knows that). */
export function ModeChip({ mode }: { mode: Mode }) {
  const { source, versions } = styleOfMode(mode)
  return <StyleChip source={source} versions={versions} />
}
