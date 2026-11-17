import { useState } from 'react'
import { Loader2 } from 'lucide-react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { GithubLoginStart } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { useJob } from '@/hooks/useJob'
import { Button } from '@/components/ui/button'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { CopyButton } from '@/components/CopyButton'
import { ErrorNotice } from '@/components/ErrorNotice'

/** GitHub sign-in status plus the device-code flow (show the one-time code and link, wait for approval). */
export function GithubSignIn({ onSignedIn }: { onSignedIn?: () => void }) {
  const qc = useQueryClient()
  const status = useQuery({ queryKey: ['github'], queryFn: api.github.status })
  const [start, setStart] = useState<GithubLoginStart | null>(null)
  const [error, setError] = useState<unknown>(null)
  const blocked = whyNot(useRole(), 'admin')
  const job = useJob(start?.job ?? null, (j) => {
    if (j.state === 'done') {
      void qc.invalidateQueries({ queryKey: ['github'] })
      void qc.invalidateQueries({ queryKey: ['github-owners'] })
      onSignedIn?.()
    }
  })
  const begin = async () => {
    setError(null)
    try {
      setStart(await api.github.login())
    } catch (e) {
      setError(e)
    }
  }
  if (status.isLoading) return <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground"><Loader2 className="size-4 animate-spin" aria-hidden /> Checking GitHub...</p>
  const state = status.data?.state ?? (status.data?.signed_in ? 'ready' : 'signed_out')
  if (status.data?.signed_in && state !== 'missing_scopes') return <div className="flex items-center gap-2 text-sm"><Badge tone="success" dot>Signed in</Badge><span>as {status.data.user}</span></div>
  // `gh` itself is missing: signing in cannot work yet. Say so, with what the panel offers to fix it.
  if (state === 'gh_missing') {
    return (
      <div className="space-y-3">
        <div className="flex items-center gap-2 text-sm"><Badge tone="amber" dot>GitHub CLI not installed</Badge></div>
        <ErrorNotice error={status.data?.problem ?? 'The GitHub CLI (gh) is not installed. Kora needs it to work with GitHub.'} />
        <Button onClick={() => void status.refetch()}>Check again</Button>
      </div>
    )
  }
  return (
    <div className="space-y-3">
      {state === 'missing_scopes' ? (
        <>
          <div className="flex items-center gap-2 text-sm"><Badge tone="amber" dot>More permissions needed</Badge><span>signed in as {status.data?.user}</span></div>
          <p className="text-sm text-muted-foreground">{status.data?.problem?.message ?? 'Sign in again to give Kora the permissions it needs.'}</p>
        </>
      ) : (
        <div className="flex items-center gap-2 text-sm"><Badge tone="neutral" dot>Not signed in</Badge></div>
      )}
      {!start && <Button onClick={begin} disabled={!!blocked} title={blocked ?? undefined}>{state === 'missing_scopes' ? 'Sign in to GitHub again' : 'Sign in to GitHub'}</Button>}
      {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
      {start && job.phase === 'running' && (
        <Alert tone="info" title="Finish signing in on GitHub">
          <ol className="list-decimal space-y-1 pl-5">
            <li>Open <a className="text-info underline" href={start.url} target="_blank" rel="noopener noreferrer">{start.url}</a></li>
            <li>Type this one-time code: <code className="mx-1 rounded bg-ink px-2 py-0.5 font-mono text-base tracking-widest">{start.code}</code> <CopyButton text={start.code} /></li>
            <li>Come back here. This page notices by itself when you have approved it.</li>
          </ol>
        </Alert>
      )}
      {job.phase === 'failed' && <ErrorNotice error={job.error} title="Sign-in did not finish" />}
      {job.phase === 'failed' && <Button onClick={begin} disabled={!!blocked} title={blocked ?? undefined}>Try again</Button>}
      <ErrorNotice error={error} />
    </div>
  )
}
