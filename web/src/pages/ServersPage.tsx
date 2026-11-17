import { useEffect, useState } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { FolderPlus, Plus, RefreshCw } from 'lucide-react'
import { api } from '@/api/client'
import type { ServerSummary } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Skeleton } from '@/components/Skeleton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { AddExistingDialog } from '@/components/AddExistingDialog'
import { ModeChip, StatusChip, UpdateChip } from '@/components/Status'
import { ServerControls } from '@/components/ServerControls'
import { CopyConnectButton, OpenFolderButton, TxAdminButton } from '@/components/ServerLinks'
import { useServerControl } from '@/hooks/useServerControl'
import { usePanelAlerts } from '@/hooks/usePanelAlerts'
import { Badge } from '@/components/ui/badge'
import { useInstallBlock } from './install/access'
import { TidyUpBanner } from '@/components/Shortcuts'

/** Asked once per browser tab: a first visit with no servers goes straight to the install wizard. */
const FIRST_RUN_KEY = 'kora.firstRunShown'

function Row({ s, alerts }: { s: ServerSummary; alerts: number }) {
  const ctl = useServerControl(s.name)
  return (
    <tr className="border-t border-border align-middle">
      <td className="px-4 py-3">
        <Link to={`/servers/${encodeURIComponent(s.name)}`} className="font-medium hover:underline">{s.name}</Link>
        {alerts > 0 && <Link to={`/servers/${encodeURIComponent(s.name)}?tab=monitoring`} className="ml-2 align-middle"><Badge tone="danger">{alerts === 1 ? '1 alert' : `${alerts} alerts`}</Badge></Link>}
        {!s.managed && <div className="text-xs text-muted-foreground">Added from an existing folder</div>}
      </td>
      <td className="px-4 py-3"><ModeChip mode={s.mode} /></td>
      <td className="px-4 py-3"><StatusChip status={s.status} pending={ctl.pending} /></td>
      <td className="px-4 py-3 font-mono text-xs">{s.ports.game}</td>
      <td className="px-4 py-3 font-mono text-xs">{s.ports.txadmin}</td>
      <td className="px-4 py-3"><UpdateChip badge={s.badge} to={`/servers/${encodeURIComponent(s.name)}?tab=resources`} /></td>
      <td className="px-4 py-3"><ServerControls name={s.name} status={s.status} busy={ctl.busy} run={ctl.run} /></td>
      <td className="px-4 py-3">
        <div className="flex flex-wrap gap-2">
          <TxAdminButton port={s.ports.txadmin} up={s.status === 'running'} />
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
  const running = servers.filter((x) => x.status === 'running').length
  const stopped = servers.filter((x) => x.status === 'stopped').length
  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="font-heading text-2xl font-bold">Your servers</h1>
        <div className="flex gap-2">
          <Button onClick={() => void q.refetch()} disabled={q.isFetching}><RefreshCw className={q.isFetching ? 'animate-spin' : ''} /> Refresh</Button>
          <Button onClick={() => setAdding(true)}><FolderPlus /> Add an existing server</Button>
        </div>
      </div>
      <TidyUpBanner />
      {q.isError && <ErrorNotice error={q.error} />}
      {q.isLoading && <Skeleton className="h-40" />}
      {q.isSuccess && servers.length === 0 && (
        <Card>
          <CardContent className="flex flex-col items-start gap-4 p-8">
            <h2 className="font-heading text-xl font-semibold">No servers yet</h2>
            <p className="max-w-prose text-muted-foreground">Kora can set up a new FiveM server for you in a few minutes: the server files, database and resources. Or add one you already have.</p>
            <Button variant="default" disabled={!!installBlock} title={installBlock?.reason} aria-describedby={installBlock ? 'install-blocked' : undefined} onClick={() => nav('/install')}><Plus /> Install your first server</Button>
            {installBlock && <p id="install-blocked" className="max-w-prose text-sm text-muted-foreground">{installBlock.reason}</p>}
          </CardContent>
        </Card>
      )}
      {servers.length > 0 && (
        <p className="flex flex-wrap items-center gap-2 text-sm" aria-label="Health summary">
          <Badge tone="success" dot>{running} running</Badge>
          {stopped > 0 && <Badge tone="neutral" dot>{stopped} stopped</Badge>}
          {openAlerts.length > 0 ? <Badge tone="danger">{openAlerts.length === 1 ? '1 alert needs attention' : `${openAlerts.length} alerts need attention`}</Badge> : <Badge tone="plain">No alerts</Badge>}
        </p>
      )}
      {servers.length > 0 && (
        <Card className="overflow-x-auto">
          <table className="w-full min-w-[64rem] text-left text-sm">
            <caption className="sr-only">Your servers</caption>
            <thead className="text-xs text-muted-foreground">
              <tr>
                {['Name', 'Mode', 'Status', 'Game port', 'txAdmin port', 'Updates', 'Actions', 'Open and copy'].map((h) => (
                  <th key={h} scope="col" className="px-4 py-3 font-medium">{h}</th>
                ))}
              </tr>
            </thead>
            <tbody>{servers.map((s) => <Row key={s.name} s={s} alerts={openAlerts.filter((a) => a.server === s.name).length} />)}</tbody>
          </table>
        </Card>
      )}
      <AddExistingDialog open={adding} onOpenChange={setAdding} />
    </div>
  )
}
