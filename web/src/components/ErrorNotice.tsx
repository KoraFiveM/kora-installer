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
}

export function toNotice(e: unknown): Notice | null {
  if (!e) return null
  if (e instanceof ApiError) return { message: e.message, code: e.code, command: e.command }
  if (typeof e === 'string') return { message: e }
  if (typeof e === 'object' && e !== null && 'message' in e) {
    const j = e as JobError
    return { message: j.message, code: j.code, command: j.command }
  }
  return { message: 'Something went wrong. Please try again.' }
}

/** Shows an API error as is; `needs_elevation` shows the command to run as administrator. */
export function ErrorNotice({ error, title }: { error: unknown; title?: string }) {
  const n = toNotice(error)
  if (!n) return null
  if (n.code === 'needs_elevation') return <ElevationNotice n={n} title={title} />
  return (
    <Alert tone="danger" title={title}>
      {n.message}
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
