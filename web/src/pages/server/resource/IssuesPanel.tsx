import { useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { ExternalLink } from 'lucide-react'
import { api } from '@/api/client'
import type { ResourcePage } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { Input, Textarea } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { ActingAs } from './ActingAs'
import { t } from '@/i18n'
import '@/i18n/en/resources'

/** Open issues of this resource's repository, a way to comment, and to report a new one. */
export function IssuesPanel({ page }: { page: ResourcePage }) {
  const s = page.server.name
  const id = page.id
  const noAdmin = whyNot(useRole(), 'admin')
  const issues = useQuery({ queryKey: ['server', s, 'resource', id, 'issues'], queryFn: () => api.resources.issues(s, id), retry: false })
  const [reply, setReply] = useState<{ n: number; text: string } | null>(null)
  const [title, setTitle] = useState('')
  const [text, setText] = useState('')
  const comment = useMutation({ mutationFn: (v: { n: number; text: string }) => api.resources.issueComment(s, id, v.n, v.text), onSuccess: () => { setReply(null); void issues.refetch() } })
  const [attach, setAttach] = useState(false)
  const summary = useQuery({ queryKey: ['support-summary', s], queryFn: () => api.support.summary(s), enabled: attach, staleTime: 60_000 })
  const report = useMutation({
    mutationFn: async () => {
      // The summary is cleaned by Kora (the same removal of secrets as a support bundle); the bundle file itself is never attached.
      const extra = attach ? (summary.data ?? (await api.support.summary(s))).text : ''
      return api.feedback.report(s, { resource: id, title: title.trim(), text: extra ? `${text}\n\n${extra}` : text })
    },
    onSuccess: () => { setTitle(''); setText(''); void issues.refetch() },
  })
  return (
    <Card>
      <CardHeader><CardTitle>{t('resources.issuesPanel.issues')}</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        {issues.isLoading && <p className="text-sm text-muted-foreground" role="status">{t('resources.issuesPanel.askingGithub')}</p>}
        <ErrorNotice error={issues.error} onRetry={() => void issues.refetch()} />
        {issues.data && issues.data.issues.length === 0 && <p className="text-sm text-muted-foreground">{t('resources.issuesPanel.thereNoOpenIssues', { repo: issues.data.repo })}</p>}
        <ul className="divide-y divide-border rounded-md border border-border" aria-label={t('resources.issuesPanel.openIssues')}>
          {(issues.data?.issues ?? []).map((i) => (
            <li key={i.number} className="space-y-2 p-3 text-sm">
              <div className="flex flex-wrap items-center gap-2">
                <span className="min-w-0 flex-1"><span className="font-mono text-xs text-muted-foreground">#{i.number}</span> {i.title} <span className="text-xs text-muted-foreground">{t('resources.issuesPanel.authorCommentsCommentValue', { author: i.author, comments: i.comments })}</span> {i.labels.map((l) => <Badge key={l} tone="plain">{l}</Badge>)}</span>
                <a className="inline-flex items-center gap-1 text-xs text-info underline" href={i.url} target="_blank" rel="noopener noreferrer" aria-label={t('resources.issuesPanel.openIssueNumberGithub', { number: i.number })}>GitHub <ExternalLink className="size-3" aria-hidden /></a>
                <Button size="sm" variant="ghost" onClick={() => setReply(reply?.n === i.number ? null : { n: i.number, text: '' })}>{t('resources.issuesPanel.comment')}</Button>
              </div>
              {reply?.n === i.number && (
                <form className="space-y-2" onSubmit={(e) => { e.preventDefault(); if (reply.text.trim() && !noAdmin) comment.mutate(reply) }}>
                  <Textarea aria-label={t('resources.issuesPanel.commentIssueNumber', { number: i.number })} rows={2} value={reply.text} onChange={(e) => setReply({ n: i.number, text: e.target.value })} />
                  <Button type="submit" size="sm" disabled={!reply.text.trim() || comment.isPending || !!noAdmin} title={noAdmin ?? undefined}>{t('resources.issuesPanel.sendComment')}</Button>
                  <ActingAs did={comment.data?.acting_as} />
                </form>
              )}
            </li>
          ))}
        </ul>
        {comment.data && <Alert tone="success">{comment.data.message}</Alert>}
        <ErrorNotice error={comment.error} />
        <form className="space-y-3 border-t border-border pt-3" onSubmit={(e) => { e.preventDefault(); if (title.trim() && !noAdmin) report.mutate() }}>
          <h3 className="text-sm font-semibold">{t('resources.issuesPanel.reportNewIssue')}</h3>
          <Field label={t('resources.issuesPanel.title')}><Input value={title} onChange={(e) => setTitle(e.target.value)} /></Field>
          <Field label={t('resources.issuesPanel.whatHappened')} hint={t('resources.issuesPanel.koraAddsItsVersion')}><Textarea value={text} onChange={(e) => setText(e.target.value)} /></Field>
          <Checkbox checked={attach} onChange={(e) => setAttach(e.target.checked)} label={t('resources.issuesPanel.addShortSummaryComputer')} hint={t('resources.issuesPanel.koraVersionSystemTool')} />
          {attach && summary.data && <details><summary className="cursor-pointer text-xs text-muted-foreground">{t('resources.issuesPanel.showWhatAdded')}</summary><pre aria-label={t('resources.issuesPanel.summaryAdded')} className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-ink p-2 font-mono text-xs">{summary.data.text}</pre></details>}
          <Button type="submit" variant="default" disabled={!title.trim() || report.isPending || !!noAdmin} title={noAdmin ?? undefined}>{t('resources.issuesPanel.reportIssue')}</Button>
          <ActingAs />
          {report.data && <Alert tone="success" title={t('resources.issuesPanel.issueReported')}><a className="break-all text-info underline" href={report.data.url} target="_blank" rel="noopener noreferrer">{report.data.url}</a></Alert>}
          <ErrorNotice error={report.error} />
        </form>
      </CardContent>
    </Card>
  )
}
