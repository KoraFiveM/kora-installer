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

/** A diff, read-only, with the added and removed lines marked by a sign and a colour. */
export function DiffView({ diff }: { diff: string }) {
  return (
    <pre className="max-h-96 overflow-auto rounded-md border border-border bg-ink p-3 font-mono text-xs" tabIndex={0} aria-label="Changes, line by line">
      {diff.split('\n').map((l, i) => (
        <div key={i} className={l.startsWith('+') && !l.startsWith('+++') ? 'text-success' : l.startsWith('-') && !l.startsWith('---') ? 'text-red' : l.startsWith('@@') ? 'text-info' : undefined}>{l || ' '}</div>
      ))}
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
      <CardHeader><CardTitle>Your changes</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <ErrorNotice error={changes.error} onRetry={() => void changes.refetch()} />
        {changes.isLoading && <p className="text-sm text-muted-foreground" role="status">Looking at the folder...</p>}
        {c && c.count === 0 && <p className="text-sm text-muted-foreground">Nothing has changed in this resource's folder.</p>}
        {c && c.count > 0 && (
          <div className="space-y-2">
            <p className="text-sm">{c.count} changed file{c.count === 1 ? '' : 's'} on <code className="font-mono text-xs">{c.branch ?? 'a single commit'}</code>:</p>
            <ul className="divide-y divide-border rounded-md border border-border" aria-label="Changed files">
              {c.files.map((f) => <li key={f.path} className="flex items-center gap-2 px-3 py-1.5 text-sm"><Badge tone="plain">{f.status === 'new' ? 'New' : f.status === 'added' ? 'Added' : f.status === 'deleted' ? 'Deleted' : f.status === 'renamed' ? 'Renamed' : 'Changed'}</Badge><span className="break-all font-mono text-xs">{f.path}</span></li>)}
            </ul>
            {c.diff && (
              <div className="space-y-2">
                <Button size="sm" variant="ghost" aria-expanded={showDiff} onClick={() => setShowDiff((v) => !v)}>{showDiff ? 'Hide the changes line by line' : 'Show the changes line by line'}</Button>
                {showDiff && <DiffView diff={c.diff} />}
                {showDiff && c.diff_truncated && <p className="text-xs text-muted-foreground">The list of changes is long, so it is cut here. The files above are complete.</p>}
              </div>
            )}
            <form className="space-y-2" onSubmit={(e) => { e.preventDefault(); if (message.trim() && !noAdmin) commit.mutate() }}>
              <Field label="Save these changes with a message" hint="This makes a commit: a saved point you can describe and send to GitHub."><Textarea value={message} onChange={(e) => setMessage(e.target.value)} rows={2} /></Field>
              <Button type="submit" variant="default" disabled={!message.trim() || commit.isPending || !!noAdmin} title={noAdmin ?? undefined}>{commit.isPending && <Loader2 className="animate-spin" aria-hidden />}Save the changes (commit)</Button>
            </form>
          </div>
        )}
        <ErrorNotice error={commit.error} />
        {commit.data && <Alert tone="success">Saved {commit.data.files} file{commit.data.files === 1 ? '' : 's'} as commit <code className="font-mono text-xs">{commit.data.commit.slice(0, 7)}</code>.</Alert>}

        {c && (
          <section className="space-y-2" aria-label="Send to GitHub">
            <h3 className="text-sm font-semibold">Send to GitHub</h3>
            {!hasFork ? (
              <p className="text-sm text-muted-foreground">This is a plain copy of the original, so there is nowhere of yours to send changes. Make your own copy (a fork) first, above.</p>
            ) : (
              <>
                <p className="text-sm text-muted-foreground">{c.ahead === null ? 'This branch is not on GitHub yet.' : c.ahead === 0 ? 'Everything saved is already on GitHub.' : `${c.ahead} saved change${c.ahead === 1 ? '' : 's'} not on GitHub yet.`}{c.behind ? ` GitHub has ${c.behind} you do not have.` : ''}</p>
                <Button disabled={push.isPending || !!noAdmin || c.branch === null} title={noAdmin ?? undefined} onClick={() => push.mutate()}>{push.isPending && <Loader2 className="animate-spin" aria-hidden />}Push to {c.repos.fork}</Button>
                <ActingAs did={push.data?.acting_as} />
                {push.data && <Alert tone="success">Pushed <code className="font-mono text-xs">{push.data.branch}</code> to {push.data.to}{push.data.new_branch ? ' (a new branch there)' : ''}.</Alert>}
                <ErrorNotice error={push.error} />
              </>
            )}
          </section>
        )}

        {c && hasFork && <OpenPullRequest page={page} />}

        <section className="space-y-2" aria-label="New branch">
          <h3 className="text-sm font-semibold">Start a new branch</h3>
          <p className="text-xs text-muted-foreground">A branch is a separate line of changes. This makes one from where the resource is now, and your changes come with you.</p>
          <form className="flex flex-wrap items-end gap-2" onSubmit={(e) => { e.preventDefault(); if (branch.trim() && !noAdmin) newBranch.mutate() }}>
            <div className="min-w-56 flex-1"><Field label="Name for the new branch"><Input value={branch} onChange={(e) => setBranch(e.target.value)} placeholder="for example fix-contacts" autoComplete="off" /></Field></div>
            <Button type="submit" disabled={!branch.trim() || newBranch.isPending || !!noAdmin} title={noAdmin ?? undefined}>Make the branch</Button>
          </form>
          <ErrorNotice error={newBranch.error} />
        </section>

        <section className="space-y-2" aria-label="Recent commits">
          <h3 className="text-sm font-semibold">Recent commits{log.data?.branch ? <> on <code className="font-mono text-xs">{log.data.branch}</code></> : null}</h3>
          <ErrorNotice error={log.error} />
          {log.data && log.data.commits.length === 0 && <p className="text-sm text-muted-foreground">No commits yet.</p>}
          <ul className="divide-y divide-border text-sm">
            {(log.data?.commits ?? []).slice(0, 10).map((x) => <li key={x.sha} className="flex flex-wrap items-baseline gap-2 py-1.5"><code className="font-mono text-xs text-muted-foreground">{x.sha.slice(0, 7)}</code><span className="min-w-0 flex-1 break-words">{x.subject}</span><span className="text-xs text-muted-foreground">{x.author}, {timeAgo(x.when * 1000, Date.now())}</span></li>)}
          </ul>
        </section>
      </CardContent>
    </Card>
  )
}
