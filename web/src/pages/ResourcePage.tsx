import { Link, useParams } from 'react-router-dom'
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { ArrowLeft, ExternalLink } from 'lucide-react'
import { api, ApiError } from '@/api/client'
import type { ControlAction } from '@/api/types'
import { useServerControl } from '@/hooks/useServerControl'
import { useKept } from '@/lib/kept'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Skeleton } from '@/components/Skeleton'
import { StatusChip } from '@/components/Status'
import NotFound from '@/pages/NotFound'
import { VersionBadge } from './server/branches/VersionBadge'
import { BranchControls } from './server/branches/ResourceDialog'
import { useUpdates } from './server/useUpdates'
import { RestartNotice } from './server/resource/RestartNotice'
import { controlBlock, ResourceActions } from './server/resource/ResourceActions'
import { useRole, whyNot } from '@/hooks/useRole'

/** `/servers/:name/resources/:resource`: the one place to see and do everything about a resource. */
export default function ResourcePage() {
  const { name = '', resource = '' } = useParams()
  const qc = useQueryClient()
  // Under ['server', name] so every action that refreshes the server refreshes this page too.
  const key = ['server', name, 'resource', resource]
  const q = useQuery({ queryKey: key, queryFn: () => api.resources.get(name, resource) })
  const updates = useUpdates(name)
  const ctl = useServerControl(name)
  const noOperator = whyNot(useRole(), 'operator')
  const [ask, setAsk] = useState<ControlAction | null>(null)
  const [said, setSaid] = useKept<Record<string, { ok: boolean; text: string }>>(`control:${name}:said`, {})
  const control = useMutation({
    mutationFn: (a: ControlAction) => api.resources.control(name, resource, a),
    onSuccess: (r) => setSaid((x) => ({ ...x, [resource]: { ok: true, text: r.message } })),
    onError: (e, a) => setSaid((x) => ({ ...x, [resource]: { ok: false, text: (e as { message?: string } | null)?.message ?? `Could not ${a} ${resource}.` } })),
    onSettled: () => void qc.invalidateQueries({ queryKey: key }),
  })

  if (q.isLoading) return <Skeleton className="h-64" />
  if (q.error instanceof ApiError && q.error.status === 404) return <NotFound />
  if (q.isError || !q.data) return <ErrorNotice error={q.error} onRetry={() => void q.refetch()} />
  const page = q.data
  const block = controlBlock(page, noOperator)
  return (
    <div className="space-y-5">
      <Link to={`/servers/${encodeURIComponent(name)}?tab=resources`} className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"><ArrowLeft className="size-4" /> Resources of {name}</Link>
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="break-all font-heading text-2xl font-bold">{page.id}</h1>
        {page.branch.state !== 'pinned' && <VersionBadge branch={page.branch} />}
        <StatusChip status={page.server.running ? 'running' : 'stopped'} pending={ctl.pending} />
      </div>
      <p className="text-sm text-muted-foreground">
        {page.repository && /^https:\/\/github\.com\//.test(page.repository)
          ? <a className="inline-flex items-center gap-1 underline" href={page.repository.replace(/\.git$/, '')} target="_blank" rel="noreferrer">{page.repository.replace(/^https:\/\/github\.com\//, '').replace(/\.git$/, '')} <ExternalLink className="size-3" aria-hidden /></a>
          : page.repository}
        {' · '}resources/{page.folder}
        {page.pinned_commit ? ` · pinned to release ${page.pinned_commit.slice(0, 7)}` : page.branch.state === 'pinned' ? '' : ` · follows ${page.default_branch}`}
      </p>
      <RestartNotice
        info={page.restart}
        pending={control.isPending || ctl.busy}
        disabledReason={block?.why ?? null}
        onRestartResource={() => setAsk('restart')}
        onRestartServer={() => ctl.run('restart')}
      />
      <Card>
        <CardHeader><CardTitle>Actions</CardTitle></CardHeader>
        <CardContent>
          <ResourceActions
            page={page}
            updates={updates}
            pending={control.isPending ? control.variables : undefined}
            said={said[resource]}
            // Starting harms nothing. Stopping or restarting can interrupt players, so it is asked first, from every button.
            onControl={(a) => (a === 'start' ? control.mutate(a) : setAsk(a))}
            onStartServer={() => ctl.run('start')}
          />
        </CardContent>
      </Card>
      {page.branch.state !== 'pinned' && page.branch.state !== 'unavailable' && (
        <Card>
          <CardHeader><CardTitle>Which version of the code</CardTitle></CardHeader>
          <CardContent><BranchControls server={page.server} resource={page.branch} /></CardContent>
        </Card>
      )}
      <ConfirmDialog
        open={ask !== null}
        onOpenChange={(o) => { if (!o) setAsk(null) }}
        title={ask === 'stop' ? `Stop ${page.id}?` : `Restart ${page.id}?`}
        description={ask === 'stop'
          ? 'The resource stops for everyone on the server until it is started again. Other resources that need it may stop working. The server itself keeps running.'
          : 'The resource stops and starts again. Players may notice a short interruption in what it does. The server itself keeps running.'}
        confirmLabel={ask === 'stop' ? 'Stop resource' : 'Restart resource'}
        destructive={ask === 'stop'}
        onConfirm={() => {
          if (ask) control.mutate(ask)
          setAsk(null)
        }}
      />
      {page.branch.note && <p className="text-sm text-muted-foreground">{page.branch.note}</p>}
    </div>
  )
}
