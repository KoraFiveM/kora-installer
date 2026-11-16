import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { GithubLoginStart } from '@/api/types'
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
  if (status.isLoading) return <p className="text-sm text-muted-foreground">Checking GitHub...</p>
  if (status.data?.signed_in) return <div className="flex items-center gap-2 text-sm"><Badge tone="success" dot>Signed in</Badge><span>as {status.data.user}</span></div>
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2 text-sm"><Badge tone="neutral" dot>Not signed in</Badge></div>
      {!start && <Button onClick={begin}>Sign in to GitHub</Button>}
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
      {job.phase === 'failed' && <Button onClick={begin}>Try again</Button>}
      <ErrorNotice error={error} />
    </div>
  )
}
