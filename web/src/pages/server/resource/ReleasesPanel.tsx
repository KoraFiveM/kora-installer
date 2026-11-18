import { useQuery } from '@tanstack/react-query'
import { ExternalLink } from 'lucide-react'
import { api } from '@/api/client'
import type { ResourcePage } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ErrorNotice } from '@/components/ErrorNotice'
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/resources/releasesPanel'

/** Releases of this resource: which one the server is pinned to, what newer ones exist, and their notes. */
export function ReleasesPanel({ page }: { page: ResourcePage }) {
  const s = page.server.name
  const id = page.id
  const rel = useQuery({ queryKey: ['server', s, 'resource', id, 'releases'], queryFn: () => api.resources.releases(s, id), retry: false })
  const newer = (rel.data?.releases ?? []).filter((r) => r.newer).length
  return (
    <Card>
      <CardHeader><CardTitle>{t('resources.releasesPanel.releases')}</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        {rel.isLoading && <p className="text-sm text-muted-foreground" role="status">{t('resources.releasesPanel.askingGithub')}</p>}
        <ErrorNotice error={rel.error} onRetry={() => void rel.refetch()} />
        {rel.data && (
          <>
            <p className="text-sm text-muted-foreground">
              {rel.data.pinned_commit ? <>{rich('resources.releasesPanel.serverPinnedCodeNode', { code: <code className="font-mono text-xs">{rel.data.pinned_commit.slice(0, 7)}</code>, newer })}</> : <>{t('resources.releasesPanel.serverFollowsLatestChanges', { repo: rel.data.repo })}</>}
            </p>
            {rel.data.releases.length === 0 && <p className="text-sm text-muted-foreground">{t('resources.releasesPanel.repoNoReleases', { repo: rel.data.repo })}</p>}
            <ul className="space-y-3" aria-label={t('resources.releasesPanel.releases')}>
              {rel.data.releases.map((r) => (
                <li key={r.tag} className="rounded-md border border-border p-3 text-sm">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-mono text-xs">{r.tag}</span> <span>{r.name}</span>
                    {r.pinned && <Badge tone="success">{t('resources.releasesPanel.thisServer')}</Badge>}
                    {r.newer && <Badge tone="info">{t('resources.releasesPanel.newer')}</Badge>}
                    {r.prerelease && <Badge tone="plain">{t('resources.releasesPanel.prerelease')}</Badge>}
                    <a className="ml-auto inline-flex items-center gap-1 text-xs text-info underline" href={r.url} target="_blank" rel="noopener noreferrer" aria-label={t('resources.releasesPanel.openReleaseTagGithub', { tag: r.tag })}>GitHub <ExternalLink className="size-3" aria-hidden /></a>
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
