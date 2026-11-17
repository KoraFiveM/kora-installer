import { useState } from 'react'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ErrorNotice } from '@/components/ErrorNotice'
import type { ResourceBranch, ServerDetail } from '@/api/types'
import { BulkSwitch } from './BulkSwitch'
import { ResourceDialog } from './ResourceDialog'
import { branchBadge, useBranches } from './useBranches'

/**
 * Branches and pull requests of a Development server's resources (the GitHub tab). Plain words first:
 * "the normal version", "try a pull request"; the git names are there but secondary.
 */
export function BranchesPanel({ server: s }: { server: ServerDetail }) {
  const b = useBranches(s.name)
  const [open, setOpen] = useState<ResourceBranch | null>(null)
  const rows = b.rows.filter((r) => r.state !== 'pinned')
  const away = rows.filter((r) => r.state !== 'default' && r.state !== 'unavailable')
  return (
    <div className="space-y-5">
      <Card>
        <CardHeader><CardTitle>Branches and pull requests</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <p className="text-sm text-muted-foreground">Each resource on this server is your own copy of it. Here you can see which version of the code each one runs, try a change someone has proposed (a pull request), or go back to the normal version.</p>
          <ErrorNotice error={b.error} onRetry={() => void b.refetch()} />
          {b.isLoading && <p className="text-sm text-muted-foreground" role="status">Looking at your resources...</p>}
          {away.length > 0 && (
            <Alert tone="info" title={`${away.length} resource${away.length === 1 ? ' is' : 's are'} not on the normal version`}>
              Updates leave {away.length === 1 ? 'it' : 'them'} alone until {away.length === 1 ? 'it goes' : 'they go'} back, so nothing is pulled into your branch by accident.
            </Alert>
          )}
          {!b.isLoading && !b.error && rows.length === 0 && <p className="text-sm text-muted-foreground">None of this server's resources can change branch.</p>}
          <ul className="divide-y divide-border" aria-label="Resources and their branches">
            {rows.map((r) => {
              const badge = branchBadge(r)
              return (
                <li key={r.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2">
                  <span className="min-w-0 flex-1 break-all font-mono text-sm">{r.id}</span>
                  <Badge tone={badge.tone}>{badge.text}</Badge>
                  {r.changed_count > 0 && <Badge tone="amber">{r.changed_count} changed file{r.changed_count === 1 ? '' : 's'}</Badge>}
                  {r.stashes > 0 && <Badge tone="plain">{r.stashes} kept aside</Badge>}
                  {r.state === 'unavailable' && r.note && <span className="w-full text-xs text-muted-foreground">{r.note}</span>}
                  <Button size="sm" disabled={r.state === 'unavailable'} aria-label={`Change the branch of ${r.id}`} onClick={() => setOpen(r)}>Change…</Button>
                </li>
              )
            })}
          </ul>
        </CardContent>
      </Card>
      <Card>
        <CardHeader><CardTitle>Several resources at once</CardTitle></CardHeader>
        <CardContent><BulkSwitch server={s} /></CardContent>
      </Card>
      {open && <ResourceDialog server={s} resource={b.byId.get(open.id) ?? open} onClose={() => setOpen(null)} />}
    </div>
  )
}
