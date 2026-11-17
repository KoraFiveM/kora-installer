import { useQuery } from '@tanstack/react-query'
import { ExternalLink } from 'lucide-react'
import { api } from '@/api/client'
import type { ResourcePage } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ErrorNotice } from '@/components/ErrorNotice'

/** Releases of this resource: which one the server is pinned to, what newer ones exist, and their notes. */
export function ReleasesPanel({ page }: { page: ResourcePage }) {
  const s = page.server.name
  const id = page.id
  const rel = useQuery({ queryKey: ['server', s, 'resource', id, 'releases'], queryFn: () => api.resources.releases(s, id), retry: false })
  const newer = (rel.data?.releases ?? []).filter((r) => r.newer).length
  return (
    <Card>
      <CardHeader><CardTitle>Releases</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        {rel.isLoading && <p className="text-sm text-muted-foreground" role="status">Asking GitHub...</p>}
        <ErrorNotice error={rel.error} onRetry={() => void rel.refetch()} />
        {rel.data && (
          <>
            <p className="text-sm text-muted-foreground">
              {rel.data.pinned_commit ? <>This server is pinned to <code className="font-mono text-xs">{rel.data.pinned_commit.slice(0, 7)}</code>{newer > 0 ? `, and ${newer} newer release${newer === 1 ? '' : 's'} exist${newer === 1 ? 's' : ''}.` : '. It is the newest release.'}</> : <>This server follows the latest changes, not a release. Releases of {rel.data.repo}:</>}
            </p>
            {rel.data.releases.length === 0 && <p className="text-sm text-muted-foreground">{rel.data.repo} has no releases.</p>}
            <ul className="space-y-3" aria-label="Releases">
              {rel.data.releases.map((r) => (
                <li key={r.tag} className="rounded-md border border-border p-3 text-sm">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-mono text-xs">{r.tag}</span> <span>{r.name}</span>
                    {r.pinned && <Badge tone="success">This server</Badge>}
                    {r.newer && <Badge tone="info">Newer</Badge>}
                    {r.prerelease && <Badge tone="plain">Pre-release</Badge>}
                    <a className="ml-auto inline-flex items-center gap-1 text-xs text-info underline" href={r.url} target="_blank" rel="noopener noreferrer" aria-label={`Open release ${r.tag} on GitHub`}>GitHub <ExternalLink className="size-3" aria-hidden /></a>
                  </div>
                  {r.notes && <p className="mt-2 whitespace-pre-wrap break-words text-xs text-muted-foreground">{r.notes}</p>}
                </li>
              ))}
            </ul>
          </>
        )}
      </CardContent>
    </Card>
  )
}
