import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Loader2 } from 'lucide-react'
import { api } from '@/api/client'
import type { ResourcePage } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input, Textarea } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { timeAgo } from '@/lib/ago'
import { ActingAs } from './ActingAs'
import { OpenPullRequest } from './OpenPullRequest'
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/resources'

/** A diff, read-only, with the added and removed lines marked by a sign and a colour. */
export function DiffView({ diff }: { diff: string }) {
  return (
    <pre className="max-h-96 overflow-auto rounded-md border border-border bg-ink p-3 font-mono text-xs" tabIndex={0} role="group" aria-label={t('resources.changesPanel.changesLineLine')}>
      {diff.split('\n').map((l, i) => {
        // The colour is a help, not the message: every added or removed line also says so in words (hidden from the eye) and starts with its sign.
        const added = l.startsWith('+') && !l.startsWith('+++')
        const removed = l.startsWith('-') && !l.startsWith('---')
        return (
          <div key={i} className={added ? 'text-success' : removed ? 'text-danger-text' : l.startsWith('@@') ? 'text-info' : undefined}>
            {added && <span className="sr-only">{t('resources.changesPanel.added')}</span>}
            {removed && <span className="sr-only">{t('resources.changesPanel.removed')}</span>}
            {l || ' '}
          </div>
        )
      })}
    </pre>
  )
}

/** What changed in the folder, saving it (commit), sending it to your own copy (push), a new branch, and recent commits. */
export function ChangesPanel({ page }: { page: ResourcePage }) {
  const qc = useQueryClient()
  const s = page.server.name
  const id = page.id
  const role = useRole()
  const noAdmin = whyNot(role, 'admin')
  const key = ['server', s, 'resource', id, 'changes']
  const changes = useQuery({ queryKey: key, queryFn: () => api.resources.changes(s, id) })
  const log = useQuery({ queryKey: ['server', s, 'resource', id, 'log'], queryFn: () => api.resources.log(s, id) })
  const [message, setMessage] = useState('')
  const [branch, setBranch] = useState('')
  const [showDiff, setShowDiff] = useState(false)
  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ['server', s] })
    void qc.invalidateQueries({ queryKey: ['branches', s] })
  }
  const commit = useMutation({ mutationFn: () => api.resources.commit(s, id, message.trim()), onSuccess: () => { setMessage(''); refresh() } })
  const push = useMutation({ mutationFn: () => api.resources.push(s, id), onSuccess: refresh })
  const newBranch = useMutation({ mutationFn: () => api.resources.newBranch(s, id, branch.trim()), onSuccess: () => { setBranch(''); refresh() } })
  const c = changes.data
  const hasFork = !!c?.repos.fork
  return (
    <Card>
      <CardHeader><CardTitle>{t('resources.changesPanel.yourChanges')}</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <ErrorNotice error={changes.error} onRetry={() => void changes.refetch()} />
        {changes.isLoading && <p className="text-sm text-muted-foreground" role="status">{t('resources.changesPanel.lookingFolder')}</p>}
        {c && c.count === 0 && <p className="text-sm text-muted-foreground">{t('resources.changesPanel.nothingChangedResourcesFolder')}</p>}
        {c && c.count > 0 && (
          <div className="space-y-2">
            <p className="text-sm">{rich('resources.changesPanel.countChangedFileValue', { count: c.count, code: <code className="font-mono text-xs">{c.branch ?? 'a single commit'}</code> })}</p>
            <ul className="divide-y divide-border rounded-md border border-border" aria-label={t('resources.changesPanel.changedFiles')}>
              {c.files.map((f) => <li key={f.path} className="flex items-center gap-2 px-3 py-1.5 text-sm"><Badge tone="plain">{f.status === 'new' ? t('resources.changesPanel.new') : f.status === 'added' ? t('resources.changesPanel.added2') : f.status === 'deleted' ? t('resources.changesPanel.deleted') : f.status === 'renamed' ? t('resources.changesPanel.renamed') : t('resources.changesPanel.changed')}</Badge><span className="break-all font-mono text-xs">{f.path}</span></li>)}
            </ul>
            {c.diff && (
              <div className="space-y-2">
                <Button size="sm" variant="ghost" aria-expanded={showDiff} onClick={() => setShowDiff((v) => !v)}>{showDiff ? t('resources.changesPanel.hideChangesLineLine') : t('resources.changesPanel.showChangesLineLine')}</Button>
                {showDiff && <DiffView diff={c.diff} />}
                {showDiff && c.diff_truncated && <p className="text-xs text-muted-foreground">{t('resources.changesPanel.listChangesLongCut')}</p>}
              </div>
            )}
            <form className="space-y-2" onSubmit={(e) => { e.preventDefault(); if (message.trim() && !noAdmin) commit.mutate() }}>
              <Field label={t('resources.changesPanel.saveTheseChangesMessage')} hint={t('resources.changesPanel.makesCommitSavedPoint')}><Textarea value={message} onChange={(e) => setMessage(e.target.value)} rows={2} /></Field>
              <Button type="submit" variant="default" disabled={!message.trim() || commit.isPending || !!noAdmin} title={noAdmin ?? undefined}>{rich('resources.changesPanel.nodeNodeSaveChanges', { node: commit.isPending && <Loader2 className="animate-spin" aria-hidden /> })}</Button>
            </form>
          </div>
        )}
        <ErrorNotice error={commit.error} />
        {commit.data && <Alert tone="success">{rich('resources.changesPanel.savedFilesFileValue', { files: commit.data.files, code: <code className="font-mono text-xs">{commit.data.commit.slice(0, 7)}</code> })}</Alert>}

        {c && (
          <section className="space-y-2" aria-label={t('resources.changesPanel.sendGithub')}>
            <h3 className="text-sm font-semibold">{t('resources.changesPanel.sendGithub')}</h3>
            {!hasFork ? (
              <p className="text-sm text-muted-foreground">{t('resources.changesPanel.plainCopyOriginalThere')}</p>
            ) : (
              <>
                <p className="text-sm text-muted-foreground">{c.ahead === null ? t('resources.changesPanel.branchGithubYet') : c.ahead === 0 ? t('resources.changesPanel.everythingSavedAlreadyGithub') : t('resources.changesPanel.aheadSavedChangeValue', { ahead: c.ahead })}{c.behind ? t('resources.changesPanel.githubBehindDo', { behind: c.behind }) : ''}</p>
                <Button disabled={push.isPending || !!noAdmin || c.branch === null} title={noAdmin ?? undefined} onClick={() => push.mutate()}>{rich('resources.changesPanel.nodeNodePushFork', { node: push.isPending ? <Loader2 className="animate-spin" aria-hidden /> : null, fork: c.repos.fork ?? '' })}</Button>
                <ActingAs did={push.data?.acting_as} />
                {push.data && <Alert tone="success">{rich('resources.changesPanel.pushedCodeNodeValue', { code: <code className="font-mono text-xs">{push.data.branch}</code>, to: push.data.to, value: push.data.new_branch ? t('resources.changesPanel.newBranchThere') : '' })}</Alert>}
                <ErrorNotice error={push.error} />
              </>
            )}
          </section>
        )}

        {c && hasFork && <OpenPullRequest page={page} />}

        <section className="space-y-2" aria-label={t('resources.changesPanel.newBranch')}>
          <h3 className="text-sm font-semibold">{t('resources.changesPanel.startNewBranch')}</h3>
          <p className="text-xs text-muted-foreground">{t('resources.changesPanel.branchSeparateLineChanges')}</p>
          <form className="flex flex-wrap items-end gap-2" onSubmit={(e) => { e.preventDefault(); if (branch.trim() && !noAdmin) newBranch.mutate() }}>
            <div className="min-w-56 flex-1"><Field label={t('resources.changesPanel.nameNewBranch')}><Input value={branch} onChange={(e) => setBranch(e.target.value)} placeholder={t('resources.changesPanel.exampleFixcontacts')} autoComplete="off" /></Field></div>
            <Button type="submit" disabled={!branch.trim() || newBranch.isPending || !!noAdmin} title={noAdmin ?? undefined}>{t('resources.changesPanel.makeBranch')}</Button>
          </form>
          <ErrorNotice error={newBranch.error} />
        </section>

        <section className="space-y-2" aria-label={t('resources.changesPanel.recentCommits')}>
          <h3 className="text-sm font-semibold">{rich('resources.changesPanel.recentCommitsNodeNode', { node: log.data?.branch ? <>{rich('resources.changesPanel.codeNode', { code: <code className="font-mono text-xs">{log.data.branch}</code> })}</> : null })}</h3>
          <ErrorNotice error={log.error} />
          {log.data && log.data.commits.length === 0 && <p className="text-sm text-muted-foreground">{t('resources.changesPanel.noCommitsYet')}</p>}
          <ul className="divide-y divide-border text-sm">
            {(log.data?.commits ?? []).slice(0, 10).map((x) => <li key={x.sha} className="flex flex-wrap items-baseline gap-2 py-1.5"><code className="font-mono text-xs text-muted-foreground">{x.sha.slice(0, 7)}</code><span className="min-w-0 flex-1 break-words">{x.subject}</span><span className="text-xs text-muted-foreground">{x.author}, {timeAgo(x.when * 1000, Date.now())}</span></li>)}
          </ul>
        </section>
      </CardContent>
    </Card>
  )
}
