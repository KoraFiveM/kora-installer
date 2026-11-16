import { Link } from 'react-router-dom'
import type { Badge as BadgeT, Mode, ServerStatus } from '@/api/types'
import { Badge } from '@/components/ui/badge'

export function StatusChip({ status, pending }: { status: ServerStatus; pending?: string }) {
  if (pending) return <Badge tone="amber" dot>{pending}</Badge>
  if (status === 'running') return <Badge tone="success" dot>Running</Badge>
  if (status === 'stopped') return <Badge tone="danger" dot>Stopped</Badge>
  return <Badge tone="neutral" dot>Folder missing</Badge>
}

function updateBadge(badge: BadgeT | null) {
  if (!badge) return <Badge tone="plain">Not checked</Badge>
  switch (badge.kind) {
    case 'up_to_date':
      return <Badge tone="plain">Up to date</Badge>
    case 'updates':
      return <Badge tone="info">{badge.count === 1 ? '1 update' : `${badge.count ?? 0} updates`}</Badge>
    case 'needs_attention':
      return <Badge tone="orange">Needs attention</Badge>
    case 'new_release':
      return <Badge tone="info">New release</Badge>
  }
}

/** `to`: when there is something to look at, the badge links there (the server's Resources tab). */
export function UpdateChip({ badge, to }: { badge: BadgeT | null; to?: string }) {
  const chip = updateBadge(badge)
  return to && badge && badge.kind !== 'up_to_date' ? <Link to={to} className="hover:underline">{chip}</Link> : chip
}

export function ModeChip({ mode }: { mode: Mode }) {
  return <Badge tone="plain">{mode === 'production' ? 'Production' : 'Development'}</Badge>
}
