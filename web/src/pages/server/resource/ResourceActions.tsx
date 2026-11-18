import { Link } from 'react-router-dom'
import { Download, Loader2, Play, RefreshCw, RotateCw, Square } from 'lucide-react'
import type { ControlAction, ResourcePage } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { ErrorNotice } from '@/components/ErrorNotice'
import { ResourceProgress } from '@/components/JobProgress'
import { timeAgo } from '@/lib/ago'
import { STATUS_INFO } from '../UpdatesPanel'
import type { Updates } from '../useUpdates'
import { t, msg, type Msg } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/resources/resourceActions'

/** The resource Kora sends commands through. Stopping it would cut the panel off from the server. */
export const CONSOLE_RESOURCE = 'kora_console'

const CONTROL: Record<ControlAction, { label: Msg; Icon: typeof Play }> = {
  start: { label: msg('resources.resourceActions.start'), Icon: Play },
  stop: { label: msg('resources.resourceActions.stop'), Icon: Square },
  restart: { label: msg('resources.resourceActions.restart'), Icon: RotateCw },
}

/** Why start, stop and restart are off, in words, with the fix named in `fix` (null = they work). */
export function controlBlock(page: ResourcePage, noOperator: string | null): { why: string; fix?: 'start_server' | 'console' } | null {
  if (noOperator) return { why: noOperator }
  if (!page.server.running) return { why: t('resources.resourceActions.serverRunningStartServer'), fix: 'start_server' }
  if (!page.server.console_enabled) return { why: t('resources.resourceActions.koraSendsTheseCommands'), fix: 'console' }
  if (page.server.console_waiting_for_restart) return { why: t('resources.resourceActions.liveConsoleSwitchedBut') }
  return null
}

/** Check for update, Update, Start, Stop, Restart: real buttons with progress and plain errors. */
export function ResourceActions({ page, updates: u, pending, said, onControl, onStartServer }: {
  page: ResourcePage
  updates: Updates
  /** The control action being sent right now. */
  pending?: ControlAction
  said?: { ok: boolean; text: string } | null
  /** Asks first for stop and restart (the page does), starts at once. */
  onControl: (a: ControlAction) => void
  onStartServer: () => void
}) {
  const role = useRole()
  const noOperator = whyNot(role, 'operator')
  const noAdmin = whyNot(role, 'admin')
  const id = page.id
  const row = u.byId.get(id)
  const info = row ? STATUS_INFO[row.status] : null
  const checking = u.checkingRow === id
  const updating = u.apply.phase === 'running'
  const block = controlBlock(page, noOperator)
  const canUpdate = row?.status === 'clean'
  return (
    <div className="space-y-4">
      <section aria-label={t('resources.resourceActions.updates')} className="space-y-2">
        <h3 className="text-sm font-semibold">{t('resources.resourceActions.updates')}</h3>
        <div className="flex flex-wrap items-center gap-2">
          <Button disabled={u.busy || noOperator !== null} title={noOperator ?? undefined} aria-busy={checking} onClick={() => void u.checkOne(id)}>{rich('resources.resourceActions.nodeNodeCheckUpdate', { node: checking ? <Loader2 className="animate-spin" aria-hidden /> : <RefreshCw aria-hidden /> })}</Button>
          <Button variant="default" disabled={!canUpdate || u.busy || noAdmin !== null} title={noAdmin ?? (canUpdate ? undefined : t('resources.resourceActions.checkUpdateFirst'))} aria-busy={updating} onClick={() => void u.applyOne(id)}>{rich('resources.resourceActions.nodeNodeUpdate', { node: updating ? <Loader2 className="animate-spin" aria-hidden /> : <Download aria-hidden /> })}</Button>
          {checking && <span className="text-xs text-muted-foreground" role="status">{t('resources.resourceActions.checking')}</span>}
          {!checking && info && <Badge tone={info.tone}>{t(info.text)}</Badge>}
          {row && !checking && u.checkedAt[id] !== undefined && <span className="text-xs text-muted-foreground">{t('resources.resourceActions.lastCheckedTimeago', { timeAgo: timeAgo(u.checkedAt[id], Date.now()) })}</span>}
        </div>
        {row?.status === 'clean' && <p className="text-sm text-muted-foreground">{row.changelog.length > 0 ? t('resources.resourceActions.behindNewerChangeWith', { behind: row.behind, changes: row.changelog.slice(0, 3).join('; ') }) : t('resources.resourceActions.behindNewerChangeValue', { behind: row.behind })}</p>}
        {info?.why && <p className="text-sm text-muted-foreground">{t(info.why)}</p>}
        {u.apply.phase !== 'idle' && <ResourceProgress events={u.apply.events} />}
        {u.apply.phase === 'done' && <Alert tone="success">{t('resources.resourceActions.updatedRunningServerStill')}</Alert>}
        <ErrorNotice error={u.error ?? (u.apply.phase === 'failed' ? u.apply.error : null)} />
      </section>
      <section aria-label={t('resources.resourceActions.runningServer')} className="space-y-2">
        <h3 className="text-sm font-semibold">{t('resources.resourceActions.runningServer2')}</h3>
        <div className="flex flex-wrap items-center gap-2">
          {(Object.keys(CONTROL) as ControlAction[]).map((a) => {
            const { label, Icon } = CONTROL[a]
            const why = block?.why ?? (a === 'stop' && id === CONSOLE_RESOURCE ? t('resources.resourceActions.koraUsesResourceSend') : null)
            return (
              <Button key={a} disabled={why !== null || pending !== undefined} title={why ?? undefined} aria-busy={pending === a} onClick={() => onControl(a)}>
                {pending === a ? <Loader2 className="animate-spin" aria-hidden /> : <Icon aria-hidden />} {t(label)}
              </Button>
            )
          })}
        </div>
        {block && (
          <p className="text-sm text-muted-foreground">
            {block.why}{' '}
            {block.fix === 'start_server' && <Button size="sm" variant="default" disabled={role.can('operator') === false} onClick={onStartServer}>{t('resources.resourceActions.startServer')}</Button>}
            {block.fix === 'console' && <Link className="underline" to={`/servers/${encodeURIComponent(page.server.name)}?tab=console`}>{t('resources.resourceActions.openConsoleTab')}</Link>}
          </p>
        )}
        {said && <p className={`text-sm ${said.ok ? 'text-muted-foreground' : 'text-danger-text'}`} role="status">{said.text}</p>}
      </section>
    </div>
  )
}
