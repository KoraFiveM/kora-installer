import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ResourceBranch } from '@/api/types'
import { offNormalText } from '@/components/NotNormalBadge'
import { t } from '@/i18n'
import '@/i18n/en/resources'

/** Where each resource of a server is. Local only, so it is cheap to keep fresh. */
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
    // The resource pages (and the restart-needed notice) live under ['server', name].
    void qc.invalidateQueries({ queryKey: ['server', server] })
  }
}

/** The short badge text for a resource's branch, and its tone. */
export function branchBadge(r: ResourceBranch): { text: string; tone: 'plain' | 'info' | 'amber' | 'orange' } {
  switch (r.state) {
    case 'default': return { text: t('resources.useBranches.normalVersionDefaultBranch', { default_branch: r.default_branch }), tone: 'plain' }
    case 'pr':
    case 'branch':
    case 'detached': return { text: offNormalText(r), tone: 'amber' }
    case 'pinned': return { text: t('resources.useBranches.fixedVersion'), tone: 'plain' }
    default: return { text: t('resources.useBranches.notAvailable'), tone: 'orange' }
  }
}
