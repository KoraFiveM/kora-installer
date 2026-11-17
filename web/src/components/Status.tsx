import { Link } from 'react-router-dom'
import type { Badge as BadgeT, Mode, RunMode, ServerStatus, Source, Versions } from '@/api/types'
import { styleOfMode } from '@/pages/install/form'
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

/** Where a server's resources come from and which versions it follows, in plain words ("Plain copies, released versions"). */
export function StyleChip({ source, versions }: { source: Source; versions: Versions }) {
  return (
    <Badge tone="plain" title={`${source === 'clone' ? "Plain copies of the original repositories" : 'Your own copies (forks) on GitHub'}; ${versions === 'released' ? 'pinned to released versions' : 'following the latest changes'}.`}>
      {source === 'clone' ? 'Plain copies' : 'Own copies'}, {versions === 'released' ? 'released versions' : 'latest changes'}
    </Badge>
  )
}

/** The same chip from the old `mode` word (answers from the folder scan, which only knows that). */
export function ModeChip({ mode }: { mode: Mode }) {
  const { source, versions } = styleOfMode(mode)
  return <StyleChip source={source} versions={versions} />
}

/** "In a container (podman, rootless)": only for a server that runs in a container; nothing for the others. */
export function RunChip({ mode, label }: { mode?: RunMode; label?: string | null }) {
  if (mode !== 'container') return null
  return <Badge tone="plain" title="This server runs in a container: its files are in its own folder and it restarts through the container engine.">{label ? label.charAt(0).toUpperCase() + label.slice(1) : 'In a container'}</Badge>
}
