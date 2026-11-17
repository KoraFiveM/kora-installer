import { useMemo, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { CheckCircle2, CircleDashed, ExternalLink, Loader2, XCircle } from 'lucide-react'
import { api } from '@/api/client'
import type { ChangesChoice, PullRequest, ResourcePage, SwitchRequest, SwitchResult } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { timeAgo } from '@/lib/ago'
import { ChangesDialog } from '../branches/ChangesDialog'
import { PrDetail } from './PrDetail'

const SOURCE_LABEL = { upstream: 'Original repository', fork: 'Your own copy' } as const

/** Checks in words with an icon, so the state is never only a colour. */
export function ChecksBadge({ state }: { state: PullRequest['checks'] }) {
  if (state === 'none') return <Badge tone="plain">No checks</Badge>
  if (state === 'passing') return <Badge tone="success"><CheckCircle2 className="size-3" aria-hidden />Checks passing</Badge>
  if (state === 'failing') return <Badge tone="danger"><XCircle className="size-3" aria-hidden />Checks failing</Badge>
  return <Badge tone="amber"><CircleDashed className="size-3" aria-hidden />Checks running</Badge>
}

export function ReviewBadge({ review }: { review: string }) {
  if (review === 'APPROVED') return <Badge tone="success">Approved</Badge>
  if (review === 'CHANGES_REQUESTED') return <Badge tone="danger">Changes requested</Badge>
  if (review === 'REVIEW_REQUIRED') return <Badge tone="plain">Waiting for a review</Badge>
  return null
}

const since = (iso: string) => {
  const t = Date.parse(iso)
  return Number.isNaN(t) ? '' : timeAgo(t, Date.now())
}

/**
 * Open pull requests of the original repository and of your own copy: search, details, and "Try it" (check one out on this
 * server). While one is being tried, it is shown first with how far behind its newest commit the folder is, "Get the latest
 * commits" and "Stop trying it".
 */
export function PullRequestsPanel({ page }: { page: ResourcePage }) {
  const qc = useQueryClient()
  const s = page.server.name
  const id = page.id
  const noAdmin = whyNot(useRole(), 'admin')
  const released = page.server.versions === 'released'
  const [understood, setUnderstood] = useState(false)
  const [filter, setFilter] = useState('')
  const [open, setOpen] = useState<number | null>(null)
  const [result, setResult] = useState<SwitchResult | null>(null)
  const [ask, setAsk] = useState<{ req: SwitchRequest; files: string[] } | null>(null)
  const prs = useQuery({ queryKey: ['server', s, 'resource', id, 'prs'], queryFn: () => api.branches.prs(s, id), retry: false })
  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ['server', s] })
    void qc.invalidateQueries({ queryKey: ['branches', s] })
    void qc.invalidateQueries({ queryKey: ['branch-choices', s] })
  }
  const go = useMutation({
    mutationFn: (req: SwitchRequest) => api.branches.switch(s, id, req).then((r) => ({ r, req })),
    onSuccess: ({ r, req }) => {
      if (r.status === 'needs_choice') {
        setResult(null)
        setAsk({ req, files: r.now.changed_files })
      } else {
        setResult(r)
        refresh()
      }
    },
  })
  const list = useMemo(() => {
    const q = filter.trim().toLowerCase()
    const all = prs.data?.prs ?? []
    return q ? all.filter((p) => `${p.number} ${p.title} ${p.author} ${p.branch}`.toLowerCase().includes(q)) : all
  }, [prs.data, filter])
  const trying = page.branch.state === 'pr' ? page.branch.pr : undefined
  const leaving = !!noAdmin || (released && !understood)
  const tryIt = (p: PullRequest) => go.mutate({ action: 'pr', pr: p.number, source: p.source })
  return (
    <Card>
      <CardHeader><CardTitle>Pull requests</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">A pull request is a change someone has proposed. Kora can download one so you can try it on this server before it is accepted.</p>
        {trying !== undefined && trying !== null && (
          <section className="space-y-2 rounded-md border border-amber/40 bg-amber/10 p-3" aria-label="The pull request being tried">
            <h3 className="text-sm font-semibold">You are trying pull request #{trying}</h3>
            <PrDetail page={page} number={trying} source={prs.data?.prs.find((p) => p.number === trying)?.source} tried onChanged={refresh} />
            <div className="flex flex-wrap gap-2">
              <Button disabled={go.isPending || !!noAdmin} title={noAdmin ?? undefined} onClick={() => go.mutate({ action: 'pr', pr: trying, source: prs.data?.prs.find((p) => p.number === trying)?.source })}>{go.isPending && <Loader2 className="animate-spin" aria-hidden />}Get the latest commits</Button>
              <Button variant="default" disabled={go.isPending || !!noAdmin} title={noAdmin ?? undefined} onClick={() => go.mutate({ action: 'default' })}>Stop trying it</Button>
            </div>
          </section>
        )}
        {result && <Alert tone={result.status === 'already' ? 'info' : 'success'}><p>{result.message}</p>{result.needs_restart && <p className="mt-1">The running server still has the old version until it is restarted. The notice at the top of this page has the button.</p>}</Alert>}
        <ErrorNotice error={go.error} onRetry={go.variables ? () => go.mutate(go.variables) : undefined} />

        <p className="text-xs text-amber" role="note">A pull request is someone else&apos;s code. Once you try it, it runs on your server like any of its resources, before anyone has accepted it. Only try pull requests from people you trust, and look at what they change on GitHub first.</p>
        {released && (
          <Alert tone="warning" title="This server follows released versions">
            <p>This server is meant to run only released versions. Trying a pull request makes this resource run unreleased code until you stop trying it.</p>
            <div className="mt-2"><Checkbox label="I understand: this resource will run unreleased code" checked={understood} onChange={(e) => setUnderstood(e.target.checked)} /></div>
          </Alert>
        )}

        <Input type="search" aria-label="Search pull requests" placeholder="Search by number, title or author" value={filter} onChange={(e) => setFilter(e.target.value)} />
        {prs.isLoading && <p className="text-sm text-muted-foreground" role="status">Asking GitHub...</p>}
        <ErrorNotice error={prs.error} onRetry={() => void prs.refetch()} />
        {(prs.data?.notes ?? []).map((n) => <p key={n} className="text-xs text-amber">{n}</p>)}
        {prs.data && list.length === 0 && <p className="text-sm text-muted-foreground">{filter.trim() ? 'No pull request matches your search.' : 'There are no open pull requests.'}</p>}
        <ul className="divide-y divide-border rounded-md border border-border" aria-label="Open pull requests">
          {list.map((p) => (
            <li key={`${p.source}${p.number}`} className="space-y-2 p-3 text-sm">
              <div className="flex flex-wrap items-center gap-2">
                <div className="min-w-0 flex-1">
                  <div><span className="font-mono text-xs text-muted-foreground">#{p.number}</span> {p.title}{p.draft && <> <Badge tone="plain">Draft</Badge></>}</div>
                  <div className="mt-0.5 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    <Badge tone={p.source === 'upstream' ? 'info' : 'amber'}>{SOURCE_LABEL[p.source]}</Badge>
                    <span>by {p.author}</span>
                    <span>opened {since(p.created) || 'a while ago'}, updated {since(p.updated) || 'a while ago'}</span>
                    <ChecksBadge state={p.checks} />
                    <ReviewBadge review={p.review} />
                    {p.from === null && <span className="text-orange">its fork was deleted</span>}
                  </div>
                </div>
                <a className="inline-flex items-center gap-1 text-xs text-info underline" href={p.url} target="_blank" rel="noopener noreferrer" aria-label={`Open pull request #${p.number} on GitHub`}>GitHub <ExternalLink className="size-3" aria-hidden /></a>
                <Button size="sm" variant="ghost" aria-expanded={open === p.number} onClick={() => setOpen(open === p.number ? null : p.number)}>{open === p.number ? 'Hide details' : 'Details'}</Button>
                <Button size="sm" disabled={go.isPending || leaving} title={noAdmin ?? (released && !understood ? 'Tick the box above first' : undefined)} aria-label={`Try pull request #${p.number}`} onClick={() => tryIt(p)}>Try it</Button>
              </div>
              {open === p.number && <PrDetail page={page} number={p.number} source={p.source} onChanged={refresh} />}
            </li>
          ))}
        </ul>
      </CardContent>
      <ChangesDialog
        open={!!ask}
        resources={[id]}
        files={ask?.files ?? []}
        busy={go.isPending}
        onCancel={() => setAsk(null)}
        onChoose={(choice: Exclude<ChangesChoice, 'ask'>) => {
          const req = ask?.req
          setAsk(null)
          if (req) go.mutate({ ...req, changes: choice })
        }}
      />
    </Card>
  )
}
