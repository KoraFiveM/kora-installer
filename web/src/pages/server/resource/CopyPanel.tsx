import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { Loader2 } from 'lucide-react'
import { api } from '@/api/client'
import type { ForkOutcome, ResourcePage } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ErrorNotice } from '@/components/ErrorNotice'
import { ActingAs } from './ActingAs'

/**
 * "Make my own copy (fork)" for a resource that is a plain clone, and for the whole server. Features that need your own
 * copy (push, open a pull request, keeping up with the original) wait for it.
 */
export function CopyPanel({ page }: { page: ResourcePage }) {
  const qc = useQueryClient()
  const s = page.server.name
  const noAdmin = whyNot(useRole(), 'admin')
  const [job, setJob] = useState<string | null>(null)
  const [err, setErr] = useState<unknown>(null)
  const run = useJob<ForkOutcome>(job, () => { void qc.invalidateQueries({ queryKey: ['server', s] }); void qc.invalidateQueries({ queryKey: ['servers'] }) })
  const start = async (resources?: string[]) => {
    setErr(null)
    try { setJob((await api.copies.fork(s, { resources })).job) } catch (e) { setErr(e) }
  }
  if (page.repos?.fork) {
    return (
      <p className="text-sm text-muted-foreground">Your own copy: <a className="text-info underline" href={`https://github.com/${page.repos.fork}`} target="_blank" rel="noopener noreferrer">{page.repos.fork}</a>{page.repos.original ? <>, a copy of {page.repos.original}</> : null}.</p>
    )
  }
  if (!page.repos) return null
  return (
    <Card id="own-copy">
      <CardHeader><CardTitle>Your own copy</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">This resource is a plain copy of <strong>{page.repos.original ?? 'the original'}</strong>, so you cannot send changes anywhere or open a pull request from it. Making your own copy (a fork on GitHub) keeps what you have: the same version, the same branch and any changes you made.</p>
        <div className="flex flex-wrap gap-2">
          <Button variant="default" disabled={run.phase === 'running' || !!noAdmin} title={noAdmin ?? undefined} onClick={() => void start([page.id])}>{run.phase === 'running' && <Loader2 className="animate-spin" aria-hidden />}Make my own copy (fork)</Button>
          <Button disabled={run.phase === 'running' || !!noAdmin} title={noAdmin ?? undefined} onClick={() => void start()}>Do it for the whole server</Button>
        </div>
        <ActingAs />
        <ErrorNotice error={err ?? (run.phase === 'failed' ? run.error : null)} />
        {run.phase === 'done' && run.result && (
          <Alert tone={run.result.failed.length ? 'warning' : 'success'} title={run.result.failed.length ? 'Some could not be copied' : 'Done'}>
            {run.result.forked.map((f) => <p key={f.resource}>{f.message}</p>)}
            {run.result.failed.map((f) => <p key={f.resource}>{f.resource}: {f.problem.message}</p>)}
            {run.result.server_is_fork && <p>This server now uses your own copies.</p>}
          </Alert>
        )}
      </CardContent>
    </Card>
  )
}
