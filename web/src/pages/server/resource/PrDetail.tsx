import { useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { Loader2 } from 'lucide-react'
import { api } from '@/api/client'
import type { PrSource, ResourcePage } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/input'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { ErrorNotice } from '@/components/ErrorNotice'
import { ActingAs } from './ActingAs'
import { ChecksBadge, ReviewBadge } from './PullRequestsPanel'

/**
 * One pull request in full: its checks, the files it changes, the conversation, and (with the right to) comment, approve,
 * ask for changes and merge. Everything here acts as the GitHub account this computer is signed in as, and says so.
 */
export function PrDetail({ page, number, source, tried, onChanged }: { page: ResourcePage; number: number; source?: PrSource; tried?: boolean; onChanged: () => void }) {
  const s = page.server.name
  const id = page.id
  const noAdmin = whyNot(useRole(), 'admin')
  const key = ['server', s, 'resource', id, 'pr', number, source ?? 'upstream']
  const v = useQuery({ queryKey: key, queryFn: () => api.pulls.view(s, id, number, source), retry: false })
  const [text, setText] = useState('')
  const [confirmMerge, setConfirmMerge] = useState(false)
  const done = () => { setText(''); void v.refetch(); onChanged() }
  const comment = useMutation({ mutationFn: () => api.pulls.comment(s, id, number, text.trim(), source), onSuccess: done })
  const review = useMutation({ mutationFn: (a: 'approve' | 'request_changes' | 'comment') => api.pulls.review(s, id, number, a, text.trim() || undefined, source), onSuccess: done })
  const merge = useMutation({ mutationFn: (method?: string) => api.pulls.merge(s, id, number, method, source), onSuccess: done })
  if (v.isLoading) return <p className="text-sm text-muted-foreground" role="status"><Loader2 className="mr-1 inline size-3 animate-spin" aria-hidden />Asking GitHub about #{number}...</p>
  if (v.error || !v.data) return <ErrorNotice error={v.error} onRetry={() => void v.refetch()} />
  const p = v.data
  const open = p.state === 'OPEN'
  const method = p.merge_methods[0]
  return (
    <div className="space-y-3 text-sm" aria-label={`Pull request #${number}`}>
      <div className="flex flex-wrap items-center gap-2">
        <Badge tone={open ? 'info' : 'plain'}>{p.state === 'MERGED' ? 'Merged' : p.state === 'CLOSED' ? 'Closed' : p.draft ? 'Draft' : 'Open'}</Badge>
        <ChecksBadge state={p.checks} />
        <ReviewBadge review={p.review} />
        {p.mergeable === 'CONFLICTING' && <Badge tone="danger">Has a conflict</Badge>}
        <span className="text-xs text-muted-foreground">{p.head} into {p.base} on {p.repo}</span>
      </div>
      {p.trying && tried && (
        <p className="text-sm" data-testid="trying-status">
          {p.trying.behind === 0 ? 'You have its newest commit.' : p.trying.behind === null ? 'GitHub has newer commits than this folder has downloaded. Get the latest commits.' : `GitHub has ${p.trying.behind} newer commit${p.trying.behind === 1 ? '' : 's'} than this folder. Get the latest commits.`}
          {p.trying.ahead > 0 ? ` You have made ${p.trying.ahead} of your own on top.` : ''}
        </p>
      )}
      {p.body && <p className="whitespace-pre-wrap break-words rounded-md border border-border p-2 text-muted-foreground">{p.body}</p>}
      {p.check_list.some((c) => c.state !== 'passing') && (
        <ul className="text-xs" aria-label="Checks that are not passing">
          {p.check_list.filter((c) => c.state !== 'passing').map((c) => <li key={c.name}>{c.name}: {c.state === 'failing' ? 'failing' : 'still running'}{c.url && <> (<a className="text-info underline" href={c.url} target="_blank" rel="noopener noreferrer">details</a>)</>}</li>)}
        </ul>
      )}
      <section aria-label="Files this pull request changes">
        <h4 className="text-xs font-semibold">Changes {p.files.length} file{p.files.length === 1 ? '' : 's'}</h4>
        <ul className="text-xs">
          {p.files.map((f) => <li key={f.path} className="flex gap-2"><span className="min-w-0 flex-1 break-all font-mono">{f.path}</span><span className="text-success"><span className="sr-only">added </span>+{f.additions}</span><span className="text-danger-text"><span className="sr-only">removed </span>-{f.deletions}</span></li>)}
        </ul>
      </section>
      <section aria-label="Conversation">
        <h4 className="text-xs font-semibold">Conversation</h4>
        {p.comments.length === 0 && <p className="text-xs text-muted-foreground">No comments yet.</p>}
        <ul className="space-y-2">
          {p.comments.map((c, i) => (
            <li key={i} className="rounded-md border border-border p-2">
              <div className="text-xs text-muted-foreground">{c.author}{c.kind === 'comment' ? '' : c.kind === 'approved' ? ' approved' : c.kind === 'changes_requested' ? ' asked for changes' : ' reviewed'}</div>
              {c.body && <div className="whitespace-pre-wrap break-words">{c.body}</div>}
            </li>
          ))}
        </ul>
      </section>
      {open && (
        <section className="space-y-2" aria-label="Your reply">
          <Textarea aria-label="Your comment or review" rows={2} value={text} onChange={(e) => setText(e.target.value)} placeholder="Write a comment or the reason for your review" />
          <div className="flex flex-wrap gap-2">
            <Button size="sm" disabled={!text.trim() || comment.isPending || !!noAdmin} title={noAdmin ?? undefined} onClick={() => comment.mutate()}>Comment</Button>
            <Button size="sm" disabled={review.isPending || !!noAdmin} title={noAdmin ?? undefined} onClick={() => review.mutate('approve')}>Approve</Button>
            <Button size="sm" disabled={!text.trim() || review.isPending || !!noAdmin} title={noAdmin ?? (text.trim() ? undefined : 'Write the reason first') ?? undefined} onClick={() => review.mutate('request_changes')}>Ask for changes</Button>
            <Button size="sm" variant="destructive" disabled={!p.can_merge || !method || merge.isPending || !!noAdmin} title={noAdmin ?? (!p.can_merge ? `Your GitHub account (${p.acting_as ?? 'not known'}) may not merge here` : undefined) ?? undefined} onClick={() => setConfirmMerge(true)}>Merge{method ? ` (${method})` : ''}</Button>
          </div>
          {!p.can_merge && <p className="text-xs text-muted-foreground">Merging needs write access on {p.repo}, and the GitHub account this computer is signed in as ({p.acting_as ?? 'not known'}) does not have it.</p>}
          <ActingAs did={p.acting_as} />
        </section>
      )}
      {(comment.data ?? review.data ?? merge.data) && <Alert tone="success">{(merge.data ?? review.data ?? comment.data)?.message}</Alert>}
      <ErrorNotice error={comment.error ?? review.error ?? merge.error} />
      <ConfirmDialog
        open={confirmMerge}
        onOpenChange={setConfirmMerge}
        title={`Merge pull request #${number}?`}
        description={`This merges it into ${p.base} of ${p.repo} on GitHub, for real, as ${p.acting_as ?? 'the account this computer is signed in as'}${method ? `, by ${method}` : ''}. It cannot be undone from here.`}
        confirmLabel="Merge it"
        destructive
        onConfirm={() => { setConfirmMerge(false); merge.mutate(method) }}
      />
    </div>
  )
}
