import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ExternalLink, Loader2 } from 'lucide-react'
import { api } from '@/api/client'
import type { ProblemAction } from '@/api/types'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { CopyButton, CopyField } from '@/components/CopyButton'
import { actionLabel, buttonActions, detailsText, isAdminCode, runAction, toProblem, useProblemHandlers, type ActionHandlers, type NoticeProblem } from '@/lib/problemActions'

/** Turn a command array into one line that can be pasted into a terminal. */
export function commandLine(cmd: string[]): string {
  return cmd.map((a) => (/[\s"'$`\\]/.test(a) ? `"${a.replace(/(["\\$`])/g, '\\$1')}"` : a)).join(' ')
}

export interface Notice {
  message: string
  code?: string
  command?: string[]
  /** HTTP status of an API error; 0 when Kora did not answer at all. */
  status?: number
}

export function toNotice(e: unknown): Notice | null {
  const p = toProblem(e)
  return p && { message: p.message, code: p.code, command: p.command, status: p.status }
}

const NO_ANSWER = 'Kora Panel did not answer. Check that it is still running on the server (`kora panel restart` restarts it), then reload this page.'

/** What to do next for a few common errors, added under the message (null = nothing to add). */
export function nextStep(n: Notice): string | null {
  if (n.status === 409 && n.message.includes('Another update is running')) return "Wait for it to finish, then try again. Its progress is on the server's Resources tab."
  return null
}

/** A refusal (403) that names the command to run, as `kora ...` between backticks: that command. */
function refusalCommand(n: Notice): string | null {
  if (n.status !== 403) return null
  return /`(kora [^`]+)`/.exec(n.message)?.[1] ?? null
}

/** Text with `commands` between backticks drawn as code. */
function WithCode({ text }: { text: string }) {
  return <>{text.split('`').map((part, i) => (i % 2 ? <code key={i} className="rounded bg-ink px-1.5 py-0.5 font-mono text-xs">{part}</code> : part))}</>
}

/** The heading of an administrator-rights problem. */
function adminTitle(code: string): string {
  switch (code) {
    case 'admin.declined':
      return 'Windows did not get permission'
    case 'admin.timed_out':
      return 'Nobody answered the permission prompt'
    case 'admin.remote':
      return 'Windows has to ask on the computer that runs Kora'
    case 'admin.busy':
      return 'Windows is still asking'
    case 'admin.lost':
      return 'The administrator part stopped'
    default:
      return 'This needs administrator rights'
  }
}

/** Errors where "Copy details" helps when nothing better is offered: real failures, not answers to a bad request. */
function isFailure(p: NoticeProblem): boolean {
  return ['failed', 'internal', 'network'].includes(p.code) || p.code.includes('.') || (p.status ?? 0) >= 500
}

/**
 * Shows an error. A Problem's `message`, its `details` (folded away), and one button per action
 * that has a handler (see lib/problemActions.tsx); commands to run, links and the log are drawn
 * here. Plain errors keep their wording; real failures without actions get "Try again" (when
 * `onRetry` is given) and "Copy details".
 */
export function ErrorNotice({ error, title, actions, onRetry }: {
  error: unknown
  title?: string
  /** Handlers for this page's actions (they win over the app-wide ones). */
  actions?: ActionHandlers
  /** The generic "Try again" when the error offers nothing more specific. */
  onRetry?: () => void
}) {
  const handlers = useProblemHandlers(actions)
  const [busy, setBusy] = useState<ProblemAction | null>(null)
  const p = toProblem(error)
  if (!p) return null
  const elevation = isAdminCode(p.code)
  // The request never arrived (see `request` in api/client.ts).
  const message = p.code === 'network' ? NO_ANSWER : p.message
  const step = nextStep(p)
  const commands = p.actions.filter((a): a is Extract<ProblemAction, { kind: 'show_command' }> => a.kind === 'show_command')
  // Older answers carry the command only as `command`.
  if (elevation && p.command && commands.length === 0) commands.push({ kind: 'show_command', command: p.command, elevated: true })
  const refusal = refusalCommand(p)
  const links = p.actions.filter((a): a is Extract<ProblemAction, { kind: 'open_link' }> => a.kind === 'open_link' && a.url.startsWith('https://'))
  const log = p.actions.find((a): a is Extract<ProblemAction, { kind: 'open_log' }> => a.kind === 'open_log')
  const buttons = buttonActions(p, handlers)
  // Windows can be asked (a "request_admin" button): the command is the last resort, folded away.
  const canAsk = buttons.some((a) => a.kind === 'request_admin')
  const fallback = buttons.length === 0 && (!!onRetry || isFailure(p))
  const copy = p.actions.some((a) => a.kind === 'copy_details') || fallback
  const run = async (a: ProblemAction) => {
    setBusy(a)
    try {
      await runAction(handlers, a, { problem: p, retry: onRetry ?? p.replay })
    } finally {
      setBusy(null)
    }
  }
  return (
    <Alert tone={elevation ? 'warning' : 'danger'} title={title ?? (elevation ? adminTitle(p.code) : undefined)}>
      <p><WithCode text={message} /></p>
      {step && <p className="mt-2">{step}</p>}
      {elevation && commands.length > 0 && !canAsk && (
        <p className="mt-2">Kora is running without administrator rights, so it cannot do this itself. Run the command below as administrator (on Windows, open a terminal with &quot;Run as administrator&quot;; on Linux, run it as root, for example with sudo), then come back here.</p>
      )}
      {canAsk && commands.length > 0 ? (
        <details className="mt-2">
          <summary className="cursor-pointer text-sm">Show me the command</summary>
          <p className="mt-1 text-sm">Or do it yourself: open a terminal with &quot;Run as administrator&quot; on the computer that runs Kora, run this command, then come back here.</p>
          {commands.map((c, i) => <CommandField key={i} command={c.command} elevated={!!c.elevated} />)}
        </details>
      ) : commands.map((c, i) => <CommandField key={i} command={c.command} elevated={!!c.elevated} />)}
      {refusal && commands.length === 0 && (
        <div className="mt-2">
          <CopyField value={refusal} label="Command to run" />
        </div>
      )}
      {links.length > 0 && (
        <ul className="mt-2 space-y-1">
          {links.map((l) => (
            <li key={l.url}><a className="inline-flex items-center gap-1 text-info underline" href={l.url} target="_blank" rel="noopener noreferrer"><ExternalLink className="size-3.5" aria-hidden />{actionLabel(l)}</a></li>
          ))}
        </ul>
      )}
      {p.details && (
        <details className="mt-2">
          <summary className="cursor-pointer text-xs text-muted-foreground">Show details</summary>
          <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-ink p-2 font-mono text-xs">{p.details}</pre>
        </details>
      )}
      {log && <LogBox job={log.job} path={log.path} />}
      {(buttons.length > 0 || fallback) && (
        <div className="mt-3 flex flex-wrap gap-2">
          {buttons.map((a, i) => (
            <Button key={`${a.kind}-${i}`} size="sm" variant={i === 0 ? 'default' : 'secondary'} disabled={busy !== null} aria-busy={busy === a} onClick={() => void run(a)}>
              {busy === a && <Loader2 className="animate-spin" aria-hidden />}
              {actionLabel(a, p)}
            </Button>
          ))}
          {fallback && onRetry && <Button size="sm" variant="default" onClick={onRetry}>Try again</Button>}
          {copy && <CopyButton text={detailsText(p)} label="Copy details" />}
        </div>
      )}
      {!(buttons.length > 0 || fallback) && copy && (
        <div className="mt-3"><CopyButton text={detailsText(p)} label="Copy details" /></div>
      )}
    </Alert>
  )
}

/** A command with a copy button. The backend sends `["kora", ...]` without `sudo`; a command to run as administrator gets `sudo ` in front on Linux. */
function CommandField({ command, elevated }: { command: string[]; elevated: boolean }) {
  return (
    <div className="mt-2">
      {elevated ? <ElevatedCommand command={command} /> : <CopyField value={commandLine(command)} label="Command to run" />}
    </div>
  )
}

function ElevatedCommand({ command }: { command: string[] }) {
  const sys = useQuery({ queryKey: ['system'], queryFn: api.system, staleTime: 60_000 })
  const prefix = sys.data?.platform === 'linux' && command[0] !== 'sudo' ? 'sudo ' : ''
  return <CopyField value={prefix + commandLine(command)} label="Command to run" />
}

/** "Show the log": the end of a panel job's log, or where the log file is. */
function LogBox({ job, path }: { job?: string; path?: string }) {
  const [open, setOpen] = useState(false)
  const [lines, setLines] = useState<string[] | null>(null)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    if (!open || !job || lines) return
    let gone = false
    api.installs.log(job).then((l) => !gone && setLines(l), () => !gone && setFailed(true))
    return () => {
      gone = true
    }
  }, [open, job, lines])
  if (!job) return path ? <p className="mt-2 text-xs">The log is in <code className="font-mono">{path}</code></p> : null
  return (
    <div className="mt-2">
      <Button variant="link" size="sm" className="h-auto p-0 text-xs" aria-expanded={open} onClick={() => setOpen(!open)}>{open ? 'Hide the log' : 'Show the log'}</Button>
      {open && (
        <div className="mt-1 space-y-1">
          {lines ? <pre aria-label="Install log" className="max-h-64 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-ink p-2 font-mono text-xs">{lines.join('\n')}</pre>
            : failed ? <p className="text-xs">The log could not be read.</p>
            : <p role="status" className="text-xs text-muted-foreground">Reading the log...</p>}
          {path && <p className="text-xs">Saved in <code className="font-mono">{path}</code></p>}
        </div>
      )}
    </div>
  )
}
