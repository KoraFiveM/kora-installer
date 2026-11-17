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
import { t as tt, msg } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/resources/pullRequestsPanel'

const SOURCE_LABEL = { upstream: msg('resources.pullRequestsPanel.originalRepository'), fork: msg('resources.pullRequestsPanel.ownCopy') } as const

/** Checks in words with an icon, so the state is never only a colour. */
export function ChecksBadge({ state }: { state: PullRequest['checks'] }) {
  if (state === 'none') return <Badge tone="plain">{tt('resources.pullRequestsPanel.noChecks')}</Badge>
  if (state === 'passing') return <Badge tone="success">{rich('resources.pullRequestsPanel.checkcircleNodeChecksPassing', { checkCircle2: <CheckCircle2 className="size-3" aria-hidden /> })}</Badge>
  if (state === 'failing') return <Badge tone="danger">{rich('resources.pullRequestsPanel.xcircleNodeChecksFailing', { xCircle: <XCircle className="size-3" aria-hidden /> })}</Badge>
  return <Badge tone="amber">{rich('resources.pullRequestsPanel.circledashedNodeChecksRunning', { circleDashed: <CircleDashed className="size-3" aria-hidden /> })}</Badge>
}

export function ReviewBadge({ review }: { review: string }) {
  if (review === 'APPROVED') return <Badge tone="success">{tt('resources.pullRequestsPanel.approved')}</Badge>
  if (review === 'CHANGES_REQUESTED') return <Badge tone="danger">{tt('resources.pullRequestsPanel.changesRequested')}</Badge>
  if (review === 'REVIEW_REQUIRED') return <Badge tone="plain">{tt('resources.pullRequestsPanel.waitingReview')}</Badge>
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
      <CardHeader><CardTitle>{tt('resources.pullRequestsPanel.pullRequests')}</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">{tt('resources.pullRequestsPanel.pullRequestChangeSomeone')}</p>
        {trying !== undefined && trying !== null && (
          <section className="space-y-2 rounded-md border border-amber/40 bg-amber/10 p-3" aria-label={tt('resources.pullRequestsPanel.pullRequestBeingTried')}>
            <h3 className="text-sm font-semibold">{tt('resources.pullRequestsPanel.tryingPullRequestTrying', { trying })}</h3>
            <PrDetail page={page} number={trying} source={prs.data?.prs.find((p) => p.number === trying)?.source} tried onChanged={refresh} />
            <div className="flex flex-wrap gap-2">
              <Button disabled={go.isPending || !!noAdmin} title={noAdmin ?? undefined} onClick={() => go.mutate({ action: 'pr', pr: trying, source: prs.data?.prs.find((p) => p.number === trying)?.source })}>{rich('resources.pullRequestsPanel.nodeNodeGetLatest', { node: go.isPending && <Loader2 className="animate-spin" aria-hidden /> })}</Button>
              <Button variant="default" disabled={go.isPending || !!noAdmin} title={noAdmin ?? undefined} onClick={() => go.mutate({ action: 'default' })}>{tt('resources.pullRequestsPanel.stopTrying')}</Button>
            </div>
          </section>
        )}
        {result && <Alert tone={result.status === 'already' ? 'info' : 'success'}><p>{result.message}</p>{result.needs_restart && <p className="mt-1">{tt('resources.pullRequestsPanel.runningServerStillOld')}</p>}</Alert>}
        <ErrorNotice error={go.error} onRetry={go.variables ? () => go.mutate(go.variables) : undefined} />

        <p className="text-xs text-amber" role="note">{tt('resources.pullRequestsPanel.pullRequestSomeoneElses')}</p>
        {released && (
          <Alert tone="warning" title={tt('resources.pullRequestsPanel.serverFollowsReleasedVersions')}>
            <p>{tt('resources.pullRequestsPanel.serverMeantRunOnly')}</p>
            <div className="mt-2"><Checkbox label={tt('resources.pullRequestsPanel.iUnderstandResourceRun')} checked={understood} onChange={(e) => setUnderstood(e.target.checked)} /></div>
          </Alert>
        )}

        <Input type="search" aria-label={tt('resources.pullRequestsPanel.searchPullRequests')} placeholder={tt('resources.pullRequestsPanel.searchNumberTitleAuthor')} value={filter} onChange={(e) => setFilter(e.target.value)} />
        {prs.isLoading && <p className="text-sm text-muted-foreground" role="status">{tt('resources.pullRequestsPanel.askingGithub')}</p>}
        <ErrorNotice error={prs.error} onRetry={() => void prs.refetch()} />
        {(prs.data?.notes ?? []).map((n) => <p key={n} className="text-xs text-amber">{n}</p>)}
        {prs.data && list.length === 0 && <p className="text-sm text-muted-foreground">{filter.trim() ? tt('resources.pullRequestsPanel.noPullRequestMatches') : tt('resources.pullRequestsPanel.thereNoOpenPull')}</p>}
        <ul className="divide-y divide-border rounded-md border border-border" aria-label={tt('resources.pullRequestsPanel.openPullRequests')}>
          {list.map((p) => (
            <li key={`${p.source}${p.number}`} className="space-y-2 p-3 text-sm">
              <div className="flex flex-wrap items-center gap-2">
                <div className="min-w-0 flex-1">
                  <div><span className="font-mono text-xs text-muted-foreground">#{p.number}</span> {p.title}{p.draft && <> <Badge tone="plain">{tt('resources.pullRequestsPanel.draft')}</Badge></>}</div>
                  <div className="mt-0.5 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    <Badge tone={p.source === 'upstream' ? 'info' : 'amber'}>{tt(SOURCE_LABEL[p.source])}</Badge>
                    <span>{tt('resources.pullRequestsPanel.byAuthor', { author: p.author })}</span>
                    <span>{tt('resources.pullRequestsPanel.openedValueUpdatedValue', { value: since(p.created) || 'a while ago', value2: since(p.updated) || 'a while ago' })}</span>
                    <ChecksBadge state={p.checks} />
                    <ReviewBadge review={p.review} />
                    {p.from === null && <span className="text-orange">{tt('resources.pullRequestsPanel.itsForkDeleted')}</span>}
                  </div>
                </div>
                <a className="inline-flex items-center gap-1 text-xs text-info underline" href={p.url} target="_blank" rel="noopener noreferrer" aria-label={tt('resources.pullRequestsPanel.openPullRequestNumber', { number: p.number })}>GitHub <ExternalLink className="size-3" aria-hidden /></a>
                <Button size="sm" variant="ghost" aria-expanded={open === p.number} onClick={() => setOpen(open === p.number ? null : p.number)}>{open === p.number ? tt('resources.pullRequestsPanel.hideDetails') : tt('resources.pullRequestsPanel.details')}</Button>
                <Button size="sm" disabled={go.isPending || leaving} title={noAdmin ?? (released && !understood ? tt('resources.pullRequestsPanel.tickBoxAboveFirst') : undefined)} aria-label={tt('resources.pullRequestsPanel.tryPullRequestNumber', { number: p.number })} onClick={() => tryIt(p)}>{tt('resources.pullRequestsPanel.tryIt')}</Button>
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
