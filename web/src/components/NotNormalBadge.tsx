import { GitBranch, GitPullRequest } from 'lucide-react'
import { Link } from 'react-router-dom'
import type { ResourceBranch } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { t } from '@/i18n'
import '@/i18n/en/core'

type Where = Pick<ResourceBranch, 'state' | 'branch' | 'pr' | 'pr_title' | 'default_branch'>

/** Is this resource somewhere other than its normal version (a branch, a pull request, a single commit)? */
export function isOffNormal(r: Pick<ResourceBranch, 'state'> | undefined | null): boolean {
  return r?.state === 'branch' || r?.state === 'pr' || r?.state === 'detached'
}

/** The words, the same everywhere: "Trying pull request #12", "On branch fix-hud". */
export function offNormalText(r: Where): string {
  if (r.state === 'pr') return r.pr_title ? t('core.notNormalBadge.tryingPullRequestTitle', { pr: r.pr ?? 0, title: r.pr_title }) : t('core.notNormalBadge.tryingPullRequestPr', { pr: r.pr ?? 0 })
  if (r.state === 'detached') return t('core.notNormalBadge.singleCommit')
  return t('core.notNormalBadge.branchBranch', { branch: r.branch ?? '' })
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
    <Badge tone="amber" title={t('core.notNormalBadge.itsNormalVersionDefault', { default_branch: branch.default_branch, text })} data-not-normal="true">
      <Icon className="size-3" aria-hidden />
      <span className="sr-only">{t('core.notNormalBadge.itsNormalVersion')}</span>
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
