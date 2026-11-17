import type { ResourceBranch } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { isOffNormal, NotNormalBadge } from '@/components/NotNormalBadge'
import { branchBadge } from './useBranches'

/** Where a resource is: the shared warning badge when it is not on its normal version, a quiet one when it is. */
export function VersionBadge({ branch, to }: { branch: ResourceBranch; to?: string }) {
  if (isOffNormal(branch)) return <NotNormalBadge branch={branch} to={to} />
  if (branch.state === 'unavailable') return <Badge tone="orange">Not available</Badge>
  const b = branchBadge(branch)
  return <Badge tone={b.tone}>{b.text}</Badge>
}
