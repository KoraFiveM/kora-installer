import { useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
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
import { useServerControl } from '@/hooks/useServerControl'

function Row({ s }: { s: ServerSummary }) {
  const ctl = useServerControl(s.name)
  return (
    <tr className="border-t border-border align-middle">
      <td className="px-4 py-3">
        <Link to={`/servers/${encodeURIComponent(s.name)}`} className="font-medium hover:underline">{s.name}</Link>
        {!s.managed && <div className="text-xs text-muted-foreground">Added from an existing folder</div>}
      </td>
      <td className="px-4 py-3"><ModeChip mode={s.mode} /></td>
      <td className="px-4 py-3"><StatusChip status={s.status} pending={ctl.pending} /></td>
      <td className="px-4 py-3 font-mono text-xs">{s.ports.game}</td>
      <td className="px-4 py-3 font-mono text-xs">{s.ports.txadmin}</td>
      <td className="px-4 py-3"><UpdateChip badge={s.badge} /></td>
      <td className="px-4 py-3"><ServerControls name={s.name} status={s.status} busy={ctl.busy} run={ctl.run} /></td>
    </tr>
  )
}

export default function ServersPage() {
  const nav = useNavigate()
  const [adding, setAdding] = useState(false)
  const q = useQuery({ queryKey: ['servers'], queryFn: api.servers.list })
  const servers = q.data ?? []
  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="font-heading text-2xl font-bold">Your servers</h1>
        <div className="flex gap-2">
          <Button onClick={() => void q.refetch()} disabled={q.isFetching}><RefreshCw className={q.isFetching ? 'animate-spin' : ''} /> Refresh</Button>
          <Button onClick={() => setAdding(true)}><FolderPlus /> Add an existing server</Button>
        </div>
      </div>
      {q.isError && <ErrorNotice error={q.error} />}
      {q.isLoading && <Skeleton className="h-40" />}
      {q.isSuccess && servers.length === 0 && (
        <Card>
          <CardContent className="flex flex-col items-start gap-4 p-8">
            <h2 className="font-heading text-xl font-semibold">No servers yet</h2>
            <p className="max-w-prose text-muted-foreground">Kora can set up a new FiveM server for you in a few minutes: the server files, database and resources. Or add one you already have.</p>
            <Button variant="default" onClick={() => nav('/install')}><Plus /> Install your first server</Button>
          </CardContent>
        </Card>
      )}
      {servers.length > 0 && (
        <Card className="overflow-x-auto">
          <table className="w-full min-w-[44rem] text-left text-sm">
            <caption className="sr-only">Your servers</caption>
            <thead className="text-xs text-muted-foreground">
              <tr>
                {['Name', 'Mode', 'Status', 'Game port', 'txAdmin port', 'Updates', 'Actions'].map((h) => (
                  <th key={h} scope="col" className="px-4 py-3 font-medium">{h}</th>
                ))}
              </tr>
            </thead>
            <tbody>{servers.map((s) => <Row key={s.name} s={s} />)}</tbody>
          </table>
        </Card>
      )}
      <AddExistingDialog open={adding} onOpenChange={setAdding} />
    </div>
  )
}
