import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ResourceBranch } from '@/api/types'

/** Where each resource of a Development server is. Local only, so it is cheap to keep fresh. */
export function useBranches(server: string, enabled = true) {
  const q = useQuery({ queryKey: ['branches', server], queryFn: () => api.branches.list(server), enabled })
  const byId = new Map<string, ResourceBranch>((q.data ?? []).map((r) => [r.id, r]))
  return { ...q, rows: q.data ?? [], byId }
}

/** Ask for the lists to be read again (after a switch, a stash or an update). */
export function useRefreshBranches(server: string) {
  const qc = useQueryClient()
  return () => {
    void qc.invalidateQueries({ queryKey: ['branches', server] })
    void qc.invalidateQueries({ queryKey: ['branch-choices', server] })
    void qc.invalidateQueries({ queryKey: ['snapshots', server] })
  }
}

/** The short badge text for a resource's branch, and its tone. */
export function branchBadge(r: ResourceBranch): { text: string; tone: 'plain' | 'info' | 'amber' | 'orange' } {
  switch (r.state) {
    case 'default': return { text: `Normal version (${r.default_branch})`, tone: 'plain' }
    case 'pr': return { text: `Trying PR #${r.pr}`, tone: 'info' }
    case 'branch': return { text: `Branch ${r.branch}`, tone: 'info' }
    case 'detached': return { text: 'A single commit', tone: 'amber' }
    case 'pinned': return { text: 'Fixed version', tone: 'plain' }
    default: return { text: 'Not available', tone: 'orange' }
  }
}
