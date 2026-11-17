import { useEffect, useState } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { FolderPlus, Plus, RefreshCw } from 'lucide-react'
import { api } from '@/api/client'
import type { BackupSummary, ServerSummary } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Skeleton } from '@/components/Skeleton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { AddExistingDialog } from '@/components/AddExistingDialog'
import { StatusChip, StyleChip, UpdateChip } from '@/components/Status'
import { ServerControls } from '@/components/ServerControls'
import { CopyConnectButton, OpenFolderButton, TxAdminButton } from '@/components/ServerLinks'
import { useServerControl } from '@/hooks/useServerControl'
import { usePanelAlerts } from '@/hooks/usePanelAlerts'
import { Badge } from '@/components/ui/badge'
import { BackupChip, useBackupLines } from '@/components/BackupChip'
import { OffNormalSummary } from '@/components/OffNormalSummary'
import { useInstallBlock } from './install/access'
import { TidyUpBanner } from '@/components/Shortcuts'
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/servers'

/** Asked once per browser tab: a first visit with no servers goes straight to the install wizard. */
const FIRST_RUN_KEY = 'kora.firstRunShown'

function Row({ s, alerts, backup }: { s: ServerSummary; alerts: number; backup?: BackupSummary }) {
  const ctl = useServerControl(s.name)
  return (
    <tr className="border-t border-border align-middle">
      <td className="px-4 py-3">
        <Link to={`/servers/${encodeURIComponent(s.name)}`} className="font-medium hover:underline">{s.name}</Link>
        {alerts > 0 && <Link to={`/servers/${encodeURIComponent(s.name)}?tab=monitoring`} className="ml-2 align-middle"><Badge tone="danger">{t('servers.serversPage.alertsCount', { count: alerts })}</Badge></Link>}
        <div className="mt-1"><OffNormalSummary server={s} /></div>
        {!s.managed && <div className="text-xs text-muted-foreground">{t('servers.serversPage.addedFromExistingFolder')}</div>}
      </td>
      <td className="px-4 py-3"><StyleChip source={s.source} versions={s.versions} /></td>
      <td className="px-4 py-3"><StatusChip status={s.status} pending={ctl.pending} /></td>
      <td className="px-4 py-3 font-mono text-xs">{s.ports.game}</td>
      <td className="px-4 py-3 font-mono text-xs">{s.ports.txadmin}</td>
      <td className="px-4 py-3"><UpdateChip badge={s.badge} to={`/servers/${encodeURIComponent(s.name)}?tab=resources`} /></td>
      <td className="px-4 py-3"><BackupChip server={s.name} summary={backup} /></td>
      <td className="px-4 py-3"><ServerControls name={s.name} status={s.status} busy={ctl.busy} run={ctl.run} /></td>
      <td className="px-4 py-3">
        <div className="flex flex-wrap gap-2">
          <TxAdminButton name={s.name} port={s.ports.txadmin} up={s.status === 'running'} />
          <OpenFolderButton name={s.name} dir={s.dir} />
          <CopyConnectButton connect={s.connect} />
        </div>
      </td>
    </tr>
  )
}

export default function ServersPage() {
  const nav = useNavigate()
  const installBlock = useInstallBlock()
  const [params, setParams] = useSearchParams()
  const [adding, setAddingState] = useState(params.get('add') === '1')
  const setAdding = (v: boolean) => {
    setAddingState(v)
    if (!v && params.has('add')) setParams({}, { replace: true })
  }
  const q = useQuery({ queryKey: ['servers'], queryFn: api.servers.list })
  const servers = q.data ?? []
  const empty = q.isSuccess && servers.length === 0
  useEffect(() => {
    // Someone who asked to add an existing server is not sent to the wizard.
    if (!empty || installBlock || params.get('add') === '1') return
    try {
      if (sessionStorage.getItem(FIRST_RUN_KEY)) return
      sessionStorage.setItem(FIRST_RUN_KEY, '1')
    } catch { /* storage blocked: still go to the wizard once per load */ }
    nav('/install', { replace: true })
  }, [empty, installBlock, nav, params])
  const { open: openAlerts } = usePanelAlerts()
  const backups = useBackupLines()
  const running = servers.filter((x) => x.status === 'running').length
  const stopped = servers.filter((x) => x.status === 'stopped').length
  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="font-heading text-2xl font-bold">{t('servers.serversPage.yourServers')}</h1>
        <div className="flex flex-wrap gap-2">
          <Button onClick={() => void q.refetch()} disabled={q.isFetching}>{rich('servers.serversPage.refreshcwNodeRefresh', { refreshCw: <RefreshCw className={q.isFetching ? 'animate-spin' : ''} /> })}</Button>
          <Button onClick={() => setAdding(true)}>{rich('servers.serversPage.folderplusNodeAddExisting', { folderPlus: <FolderPlus /> })}</Button>
        </div>
      </div>
      <TidyUpBanner />
      {q.isError && <ErrorNotice error={q.error} />}
      {q.isLoading && <Skeleton className="h-40" />}
      {q.isSuccess && servers.length === 0 && (
        <Card>
          <CardContent className="flex flex-col items-start gap-4 p-8">
            <h2 className="font-heading text-xl font-semibold">{t('servers.serversPage.noServersYet')}</h2>
            <p className="max-w-prose text-muted-foreground">{t('servers.serversPage.koraSetUpNew')}</p>
            <Button variant="default" disabled={!!installBlock} title={installBlock?.reason} aria-describedby={installBlock ? 'install-blocked' : undefined} onClick={() => nav('/install')}>{rich('servers.serversPage.plusNodeInstallFirst', { plus: <Plus /> })}</Button>
            {installBlock && <p id="install-blocked" className="max-w-prose text-sm text-muted-foreground">{installBlock.reason}</p>}
          </CardContent>
        </Card>
      )}
      {servers.length > 0 && (
        <p className="flex flex-wrap items-center gap-2 text-sm" aria-label={t('servers.serversPage.healthSummary')}>
          <Badge tone="success" dot>{t('servers.serversPage.runningRunning', { running })}</Badge>
          {stopped > 0 && <Badge tone="neutral" dot>{t('servers.serversPage.stoppedStopped', { stopped })}</Badge>}
          {openAlerts.length > 0 ? <Badge tone="danger">{t('servers.serversPage.alertsNeedAttention', { count: openAlerts.length })}</Badge> : <Badge tone="plain">{t('servers.serversPage.noAlerts')}</Badge>}
        </p>
      )}
      {servers.length > 0 && (
        <Card className="overflow-x-auto">
          <table className="w-full min-w-[72rem] text-left text-sm">
            <caption className="sr-only">{t('servers.serversPage.yourServers')}</caption>
            <thead className="text-xs text-muted-foreground">
              <tr>
                {['Name', 'Mode', 'Status', t('servers.serversPage.gamePort'), 'txAdmin port', 'Updates', t('servers.serversPage.lastBackup'), 'Actions', t('servers.serversPage.openCopy')].map((h) => (
                  <th key={h} scope="col" className="px-4 py-3 font-medium">{h}</th>
                ))}
              </tr>
            </thead>
            <tbody>{servers.map((s) => <Row key={s.name} s={s} alerts={openAlerts.filter((a) => a.server === s.name).length} backup={backups.by.get(s.name)} />)}</tbody>
          </table>
        </Card>
      )}
      <AddExistingDialog open={adding} onOpenChange={setAdding} />
    </div>
  )
}
