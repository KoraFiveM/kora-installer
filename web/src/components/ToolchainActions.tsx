/**
 * The fix buttons that work anywhere a Problem carries them (install wizard, GitHub tab, System check):
 *
 * - `install_tool` ("Install it for me"): downloads Kora's own copy of Git or the GitHub CLI, with a progress bar;
 * - `sign_in_github` ("Sign in to GitHub" / "Sign in again"): the panel's one-time-code sign-in;
 * - `start_database` ("Start it for me"): starts the installed database server.
 *
 * Each opens a small dialog that does the work, shows progress, and says plainly what happened. Wrap the app
 * in `<ToolchainActions>` once; it registers the handlers with `<ProblemActions>` (see lib/problemActions.tsx).
 */
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { CheckCircle2, Loader2 } from 'lucide-react'
import { api } from '@/api/client'
import type { JobStarted, ToolInstallEvent } from '@/api/types'
import { useJob } from '@/hooks/useJob'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Progress } from '@/components/ui/progress'
import { ErrorNotice } from '@/components/ErrorNotice'
import { GithubSignIn } from '@/components/GithubSignIn'
import { ProblemActions, type ActionHandlers } from '@/lib/problemActions'

type Request = (
  | { kind: 'install'; tool: 'git' | 'gh' }
  | { kind: 'signin' }
  | { kind: 'database' }
) & { id: number; afterFix?: () => void | Promise<void> }

const TOOL_NAME = { git: 'Git', gh: 'the GitHub CLI' } as const

/** What on the page may have changed once a fix has worked. */
const STALE = [['github'], ['github-owners'], ['tools'], ['system-check'], ['system']] as const

export function ToolchainActions({ children }: { children: ReactNode }) {
  const [req, setReq] = useState<Request | null>(null)
  const seq = useRef(0)
  const handlers = useMemo<ActionHandlers>(
    () => ({
      install_tool: (a, c) => {
        if (a.tool === 'git' || a.tool === 'gh') setReq({ kind: 'install', tool: a.tool, id: ++seq.current, afterFix: c.afterFix })
      },
      sign_in_github: (_, c) => setReq({ kind: 'signin', id: ++seq.current, afterFix: c.afterFix }),
      start_database: (_, c) => setReq({ kind: 'database', id: ++seq.current, afterFix: c.afterFix }),
    }),
    [],
  )
  return (
    <ProblemActions handlers={handlers}>
      {children}
      <Dialog open={!!req} onOpenChange={(o) => !o && setReq(null)}>
        {req && (
          <DialogContent title={titleOf(req)} description={descriptionOf(req)}>
            {req.kind === 'install' && <RunJob key={req.id} start={() => api.tools.install(req.tool)} working={`Setting up ${TOOL_NAME[req.tool]}`} done={`${cap(TOOL_NAME[req.tool])} is ready.`} afterFix={req.afterFix} onClose={() => setReq(null)} />}
            {req.kind === 'database' && <RunJob key={req.id} start={() => api.systemCheck.startDatabase()} working="Starting the database server" done="The database server is running." afterFix={req.afterFix} onClose={() => setReq(null)} />}
            {req.kind === 'signin' && <SignIn key={req.id} afterFix={req.afterFix} onClose={() => setReq(null)} />}
          </DialogContent>
        )}
      </Dialog>
    </ProblemActions>
  )
}

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1)

function titleOf(r: Request): string {
  if (r.kind === 'install') return `Install ${TOOL_NAME[r.tool]}`
  return r.kind === 'signin' ? 'Sign in to GitHub' : 'Start the database'
}

function descriptionOf(r: Request): string | undefined {
  if (r.kind === 'install') return 'Kora keeps its own copy in its own folder. It is checked against a known checksum first, needs no administrator rights, and changes nothing else on this computer.'
  if (r.kind === 'database') return 'The database server is installed but not running. Starting it may need administrator rights.'
  return undefined
}

function useRefreshAfter() {
  const qc = useQueryClient()
  return useCallback(() => {
    for (const k of STALE) void qc.invalidateQueries({ queryKey: [...k] })
  }, [qc])
}

/** Start a job when the dialog opens, show its progress, and report how it ended. */
function RunJob({ start, working, done, afterFix, onClose }: { start: () => Promise<JobStarted>; working: string; done: string; afterFix?: () => void | Promise<void>; onClose: () => void }) {
  const refresh = useRefreshAfter()
  const [jobId, setJobId] = useState<string | null>(null)
  const [startErr, setStartErr] = useState<unknown>(null)
  const [attempt, setAttempt] = useState(0)
  const begun = useRef(-1)
  useEffect(() => {
    if (begun.current === attempt) return
    begun.current = attempt
    setJobId(null)
    setStartErr(null)
    start().then((j) => setJobId(j.job), setStartErr)
    // `start` is a new function each render; only `attempt` restarts it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [attempt])
  const carried = useRef(false)
  const job = useJob<unknown, ToolInstallEvent>(jobId, (j) => {
    if (j.state !== 'done') return
    refresh()
    // What was waiting for this carries on by itself, and the dialog gets out of the way.
    if (afterFix && !carried.current) {
      carried.current = true
      onClose()
      void afterFix()
    }
  })
  const retry = { retry: () => setAttempt((n) => n + 1) }
  const error = startErr ?? (job.phase === 'failed' ? job.error : null)
  if (error) {
    return (
      <div className="space-y-3">
        <ErrorNotice error={error} actions={retry} onRetry={retry.retry} onFixed={() => {}} />
        <div className="flex justify-end"><Button onClick={onClose}>Close</Button></div>
      </div>
    )
  }
  if (job.phase === 'done') {
    return (
      <div className="space-y-4">
        <Alert tone="success" title="Done">{done}{afterFix ? ' Carrying on with what you were doing.' : ' If something was waiting for it, you can try that again now.'}</Alert>
        <div className="flex justify-end"><Button onClick={onClose}>Close</Button></div>
      </div>
    )
  }
  const last = [...job.events].reverse().find((e) => typeof e.message === 'string')
  const fraction = [...job.events].reverse().find((e) => typeof e.fraction === 'number')?.fraction
  return (
    <div className="space-y-3" aria-live="polite">
      <p className="flex items-center gap-2 text-sm"><Loader2 className="size-4 animate-spin text-info" aria-hidden />{last?.message ?? working}</p>
      <Progress value={Math.round((fraction ?? 0.02) * 100)} label={working} />
      <p className="text-xs text-muted-foreground">You can keep this page open; it carries on by itself.</p>
    </div>
  )
}

function SignIn({ afterFix, onClose }: { afterFix?: () => void | Promise<void>; onClose: () => void }) {
  const refresh = useRefreshAfter()
  const [done, setDone] = useState(false)
  return (
    <div className="space-y-4">
      <GithubSignIn onSignedIn={() => {
        setDone(true)
        refresh()
        if (afterFix) {
          onClose()
          void afterFix()
        }
      }} />
      {done && (
        <div className="flex items-center justify-between gap-3">
          <p className="flex items-center gap-2 text-sm"><CheckCircle2 className="size-4 text-success" aria-hidden />You are signed in.{afterFix ? ' Carrying on with what you were doing.' : ' If something was waiting for it, you can try that again now.'}</p>
          <Button onClick={onClose}>Close</Button>
        </div>
      )}
      {!done && <div className="flex justify-end"><Button variant="secondary" onClick={onClose}>Close</Button></div>}
    </div>
  )
}
