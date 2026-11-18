import { Link } from 'react-router-dom'
import { GitBranch } from 'lucide-react'
import { useQuery } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ServerSummary } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { isOffNormal, offNormalSummary } from './NotNormalBadge'

/** Where each resource of a server is (local reads only); kept a minute so a long servers list does not hammer the disk. */
export function useOffNormal(server: Pick<ServerSummary, 'name' | 'managed'>) {
  const q = useQuery({ queryKey: ['branches', server.name], queryFn: () => api.branches.list(server.name), staleTime: 60_000, enabled: server.managed })
  const rows = (q.data ?? []).filter(isOffNormal)
  return { rows, summary: offNormalSummary(q.data ?? []) }
}

/** The small "N resources not on their normal version" line, linking to the Resources tab. */
export function OffNormalSummary({ server, list }: { server: Pick<ServerSummary, 'name' | 'managed'>; list?: boolean }) {
  const { rows, summary } = useOffNormal(server)
  if (!summary) return null
  const base = `/servers/${encodeURIComponent(server.name)}`
  return (
    <div className="space-y-1">
      <Link to={`${base}?tab=resources`} className="inline-flex no-underline">
        <Badge tone="amber"><GitBranch className="size-3" aria-hidden />{summary}</Badge>
      </Link>
      {list && (
        <ul className="text-sm">
          {rows.map((r) => (
            <li key={r.id}><Link className="font-mono text-xs underline" to={`${base}/resources/${encodeURIComponent(r.id)}`}>{r.id}</Link>: {r.label}</li>
          ))}
        </ul>
      )}
    </div>
  )
}
