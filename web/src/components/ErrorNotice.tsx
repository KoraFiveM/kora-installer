import { useQuery } from '@tanstack/react-query'
import { ApiError, api } from '@/api/client'
import type { JobError } from '@/api/types'
import { Alert } from '@/components/ui/alert'
import { CopyField } from '@/components/CopyButton'

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
  if (!e) return null
  if (e instanceof ApiError) return { message: e.message, code: e.code, command: e.command, status: e.status }
  if (typeof e === 'string') return { message: e }
  if (typeof e === 'object' && e !== null && 'message' in e) {
    const j = e as JobError
    return { message: j.message, code: j.code, command: j.command }
  }
  return { message: 'Something went wrong. Please try again.' }
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

/** Shows an API error with a next step where one is known; `needs_elevation` shows the command to run as administrator. */
export function ErrorNotice({ error, title }: { error: unknown; title?: string }) {
  const n = toNotice(error)
  if (!n) return null
  if (n.code === 'needs_elevation') return <ElevationNotice n={n} title={title} />
  // The request never arrived (see `request` in api/client.ts).
  const message = n.code === 'network' ? NO_ANSWER : n.message
  const step = nextStep(n)
  const command = refusalCommand(n)
  return (
    <Alert tone="danger" title={title}>
      <p><WithCode text={message} /></p>
      {step && <p className="mt-2">{step}</p>}
      {command && (
        <div className="mt-2">
          <CopyField value={command} label="Command to run" />
        </div>
      )}
    </Alert>
  )
}

/** The backend sends `["kora", ...]` without `sudo`; on Linux the command is shown with `sudo ` in front. */
function ElevationNotice({ n, title }: { n: Notice; title?: string }) {
  const sys = useQuery({ queryKey: ['system'], queryFn: api.system, staleTime: 60_000 })
  const prefix = sys.data?.platform === 'linux' ? 'sudo ' : ''
  return (
    <Alert tone="warning" title={title ?? 'This needs administrator rights'}>
      <p>{n.message}</p>
      <p className="mt-2">Kora is running without administrator rights, so it cannot do this itself. Run the command below as administrator (on Windows, open a terminal with "Run as administrator"; on Linux, run it as root, for example with sudo), then come back here.</p>
      {n.command && (
        <div className="mt-2">
          <CopyField value={prefix + commandLine(n.command)} label="Command to run" />
        </div>
      )}
    </Alert>
  )
}
