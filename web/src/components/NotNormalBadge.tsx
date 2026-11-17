import { GitBranch, GitPullRequest } from 'lucide-react'
import { Link } from 'react-router-dom'
import type { ResourceBranch } from '@/api/types'
import { Badge } from '@/components/ui/badge'

type Where = Pick<ResourceBranch, 'state' | 'branch' | 'pr' | 'default_branch'>

/** Is this resource somewhere other than its normal version (a branch, a pull request, a single commit)? */
export function isOffNormal(r: Pick<ResourceBranch, 'state'> | undefined | null): boolean {
  return r?.state === 'branch' || r?.state === 'pr' || r?.state === 'detached'
}

/** The words, the same everywhere: "Trying pull request #12", "On branch fix-hud". */
export function offNormalText(r: Where): string {
  if (r.state === 'pr') return `Trying pull request #${r.pr}`
  if (r.state === 'detached') return 'On a single commit'
  return `On branch ${r.branch}`
}

/**
 * THE badge for a resource that is not on its normal version. One colour (the warning one), an icon and words, so it is
 * never colour alone; `title` explains it. Nothing is drawn for a resource on its normal version.
 */
export function NotNormalBadge({ branch, to }: { branch: Where | undefined | null; to?: string }) {
  if (!branch || !isOffNormal(branch)) return null
  const Icon = branch.state === 'pr' ? GitPullRequest : GitBranch
  const text = offNormalText(branch)
  const badge = (
    <Badge tone="amber" title={`Not on its normal version (${branch.default_branch}). ${text}.`} data-not-normal="true">
      <Icon className="size-3" aria-hidden />
      <span className="sr-only">Not on its normal version: </span>
      {text}
    </Badge>
  )
  return to ? <Link to={to} className="no-underline">{badge}</Link> : badge
}

/** "2 resources not on their normal version", for a server's row or Overview. Nothing for 0. */
export function offNormalSummary(rows: Array<Pick<ResourceBranch, 'state'>>): string | null {
  const n = rows.filter(isOffNormal).length
  return n === 0 ? null : `${n} resource${n === 1 ? '' : 's'} not on ${n === 1 ? 'its' : 'their'} normal version`
}
