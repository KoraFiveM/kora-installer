import { Loader2, RotateCw } from 'lucide-react'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import type { RestartInfo } from '@/api/resource'
import { RESTART_NOTICE } from '@/api/resource'

/**
 * Said whenever the files of a resource changed and the running server has not loaded them yet. The fact comes from the
 * server (changed-at against restarted-at), so it shows after a reload and for every user until a real restart.
 */
export function RestartNotice({ info, onRestartResource, onRestartServer, pending, disabledReason }: {
  info: RestartInfo
  onRestartResource: () => void
  onRestartServer: () => void
  pending: boolean
  /** Why restarting the resource is off (the server must be running with the live console on); the server restart still works. */
  disabledReason: string | null
}) {
  if (!info.needed) return null
  return (
    <Alert tone="warning" title={info.notice ?? RESTART_NOTICE}>
      {info.why && <p>What changed: {info.why}.</p>}
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <Button size="sm" variant="default" disabled={pending || disabledReason !== null} title={disabledReason ?? undefined} onClick={onRestartResource}>
          {pending ? <Loader2 className="animate-spin" aria-hidden /> : <RotateCw aria-hidden />} Restart this resource
        </Button>
        <Button size="sm" variant="ghost" disabled={pending} onClick={onRestartServer}>Restart the whole server</Button>
      </div>
      {disabledReason && <p className="mt-1 text-xs text-muted-foreground">{disabledReason}</p>}
    </Alert>
  )
}
