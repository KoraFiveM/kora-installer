import { useState } from 'react'
import { Link } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Play, Plus, RefreshCw, RotateCw, Square, Trash2 } from 'lucide-react'
import { api } from '@/api/client'
import type { ControlAction, FleetServer, HubNode } from '@/api/types'
import { useRole } from '@/hooks/useRole'
import { useAlerts } from '@/lib/app'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { ErrorNotice, toNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { Skeleton } from '@/components/Skeleton'
import { StatusChip, StyleChip, UpdateChip } from '@/components/Status'
import { useJob } from '@/hooks/useJob'

const ING: Record<ControlAction, string> = { start: 'Starting', stop: 'Stopping', restart: 'Restarting' }
const DONE: Record<ControlAction, string> = { start: 'started', stop: 'stopped', restart: 'restarted' }

/** Start / stop / restart one server of this panel or of a node, and follow the job. */
function useFleetControl(s: FleetServer) {
  const qc = useQueryClient()
  const { push } = useAlerts()
  const [job, setJob] = useState<string | null>(null)
  const [action, setAction] = useState<ControlAction | null>(null)
  const [starting, setStarting] = useState(false)
  const label = `${s.name} on ${s.node_name}`
  const view = useJob(job, (j) => {
    const a = action
    setJob(null)
    setAction(null)
    void qc.invalidateQueries({ queryKey: ['fleet'] })
    if (j.state === 'failed') push('danger', `${label}: ${j.error?.message ?? 'The action did not finish.'}`)
    else if (a) push('success', `${label} ${DONE[a]}.`)
  })
  const run = async (a: ControlAction) => {
    setStarting(true)
    setAction(a)
    try {
      const r = s.node_id === null ? await api.servers.control(s.name, a) : await api.hub.nodeControl(s.node_id, s.name, a)
      setJob(r.job)
    } catch (e) {
      setAction(null)
      push('danger', `${label}: ${toNotice(e)?.message ?? 'The action did not start.'}`)
    } finally {
      setStarting(false)
    }
  }
  const busy = starting || view.phase === 'running'
  return { run, busy, pending: busy && action ? ING[action] : undefined }
}

function Actions({ s, busy, run }: { s: FleetServer; busy: boolean; run: (a: ControlAction) => void }) {
  const [confirm, setConfirm] = useState(false)
  if (s.status === 'missing') return <span className="text-xs text-muted-foreground">Folder missing</span>
  if (s.actions.length === 0) return <span className="text-xs text-muted-foreground">Update that node to control it from here</span>
  const can = (a: ControlAction) => s.actions.includes(a)
  return (
    <div className="flex gap-2">
      {s.status === 'stopped' && can('start') && (
        <Button size="sm" disabled={busy} onClick={() => run('start')} aria-label={`Start ${s.name} on ${s.node_name}`}><Play /> Start</Button>
      )}
      {s.status === 'running' && can('stop') && (
        <Button size="sm" disabled={busy} onClick={() => run('stop')} aria-label={`Stop ${s.name} on ${s.node_name}`}><Square /> Stop</Button>
      )}
      {s.status === 'running' && can('restart') && (
        <Button size="sm" disabled={busy} onClick={() => setConfirm(true)} aria-label={`Restart ${s.name} on ${s.node_name}`}><RotateCw /> Restart</Button>
      )}
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title={`Restart ${s.name}?`}
        description={`Everyone playing on ${s.name} (${s.node_name}) will be disconnected for a minute or so while it restarts.`}
        confirmLabel="Restart server"
        onConfirm={() => {
          setConfirm(false)
          run('restart')
        }}
      />
    </div>
  )
}

function ServerRow({ s }: { s: FleetServer }) {
  const ctl = useFleetControl(s)
  return (
    <tr className="border-t border-border align-middle">
      <td className="px-4 py-3 font-medium">
        {s.node_id === null ? <Link to={`/servers/${encodeURIComponent(s.name)}`} className="hover:underline">{s.name}</Link> : s.name}
      </td>
      <td className="px-4 py-3 text-muted-foreground">{s.node_name}</td>
      <td className="px-4 py-3"><StyleChip source={s.source} versions={s.versions} /></td>
      <td className="px-4 py-3"><StatusChip status={s.status} pending={ctl.pending} /></td>
      <td className="px-4 py-3 font-mono text-xs">{s.ports.game}</td>
      <td className="px-4 py-3"><UpdateChip badge={s.badge} /></td>
      <td className="px-4 py-3"><Actions s={s} busy={ctl.busy} run={ctl.run} /></td>
    </tr>
  )
}

function AddNodeDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const qc = useQueryClient()
  const { push } = useAlerts()
  const [name, setName] = useState('')
  const [url, setUrl] = useState('')
  const [token, setToken] = useState('')
  const add = useMutation({
    mutationFn: () => api.hub.addNode({ name, url, token }),
    onSuccess: (n) => {
      void qc.invalidateQueries({ queryKey: ['fleet'] })
      push(n.insecure ? 'warning' : 'success', n.insecure ? `${n.name} was added, but ${n.warning ?? 'its address is not private.'}` : `${n.name} was added.`)
      setName('')
      setUrl('')
      setToken('')
      onOpenChange(false)
    },
  })
  return (
    <Dialog open={open} onOpenChange={(o) => { if (!o) add.reset(); onOpenChange(o) }}>
      <DialogContent title="Add a node" description="A node is another computer running Kora Panel. Create a token there with 'kora panel user add', then enter it here. Kora checks the connection before saving.">
        <form className="space-y-4" onSubmit={(e) => { e.preventDefault(); add.mutate() }}>
          <Field label="Name"><Input value={name} onChange={(e) => setName(e.target.value)} placeholder="Garage box" autoComplete="off" /></Field>
          <Field label="Address" hint="For example http://192.168.1.20:47120. Use https:// or Tailscale for anything outside your own network."><Input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="http://192.168.1.20:47120" autoComplete="off" /></Field>
          <Field label="Token" hint="Saved on this computer only, and never shown again."><Input type="password" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" /></Field>
          {add.isError && <ErrorNotice error={add.error} />}
          <div className="flex justify-end gap-2">
            <Button type="button" onClick={() => onOpenChange(false)}>Cancel</Button>
            <Button type="submit" variant="default" disabled={add.isPending || !name.trim() || !url.trim() || !token.trim()}>{add.isPending ? 'Checking...' : 'Add node'}</Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function NodeCard({ n, onRemove }: { n: HubNode; onRemove: () => void }) {
  return (
    <li className="flex flex-wrap items-start justify-between gap-3 rounded-md border border-border p-4 text-sm">
      <div className="min-w-0 space-y-1">
        <div className="flex flex-wrap items-center gap-2 font-medium">
          {n.name}
          {n.reachable ? <Badge tone="success" dot>Online</Badge> : <Badge tone="danger" dot>Not reachable</Badge>}
          {n.version && <Badge tone="plain">Kora {n.version}</Badge>}
          {n.reachable && !n.can_control && <Badge tone="orange">View only: update it to control it</Badge>}
        </div>
        <div className="truncate font-mono text-xs text-muted-foreground">{n.url}</div>
        {n.server_count !== null && <div className="text-xs text-muted-foreground">{n.server_count === 1 ? '1 server' : `${n.server_count} servers`}</div>}
        {n.last_error && <div className="text-xs text-destructive">{n.last_error}</div>}
        {n.warning && <div className="text-xs text-orange">{n.warning}</div>}
      </div>
      <Button size="sm" onClick={onRemove} aria-label={`Remove ${n.name}`}><Trash2 /> Remove</Button>
    </li>
  )
}

export default function FleetPage() {
  const { remote } = useRole()
  if (remote) {
    return (
      <div className="space-y-6">
        <h1 className="font-heading text-2xl font-bold">Fleet</h1>
        <Alert tone="info">The fleet can only be opened on the computer that runs Kora, because it uses the sign-in tokens of the other panels.</Alert>
      </div>
    )
  }
  return <Fleet />
}

function Fleet() {
  const qc = useQueryClient()
  const { push } = useAlerts()
  const [adding, setAdding] = useState(false)
  const [removing, setRemoving] = useState<HubNode | null>(null)
  const q = useQuery({ queryKey: ['fleet'], queryFn: api.hub.fleet, refetchInterval: 20000 })
  const remove = useMutation({
    mutationFn: (id: string) => api.hub.removeNode(id),
    onSuccess: () => {
      push('success', `${removing?.name ?? 'The node'} was removed from this list.`)
      setRemoving(null)
      void qc.invalidateQueries({ queryKey: ['fleet'] })
    },
  })
  const nodes = q.data?.nodes ?? []
  const servers = q.data?.servers ?? []
  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="font-heading text-2xl font-bold">Fleet</h1>
        <div className="flex gap-2">
          <Button onClick={() => void q.refetch()} disabled={q.isFetching}><RefreshCw className={q.isFetching ? 'animate-spin' : ''} /> Refresh</Button>
          <Button onClick={() => setAdding(true)}><Plus /> Add a node</Button>
        </div>
      </div>
      {q.isError && <ErrorNotice error={q.error} />}
      {q.data?.partial && <Alert tone="warning" title="Some nodes are not answering">The list below shows what could be reached. Check the nodes marked "Not reachable".</Alert>}
      {q.isLoading && <Skeleton className="h-40" />}
      {q.isSuccess && (
        <Card>
          <CardHeader><CardTitle>Nodes</CardTitle></CardHeader>
          <CardContent>
            {nodes.length === 0 ? (
              <p className="text-sm text-muted-foreground">No other panels yet. Add a node to see and control its servers from here.</p>
            ) : (
              <ul className="space-y-3">{nodes.map((n) => <NodeCard key={n.id} n={n} onRemove={() => setRemoving(n)} />)}</ul>
            )}
          </CardContent>
        </Card>
      )}
      {servers.length > 0 && (
        <Card className="overflow-x-auto">
          <table className="w-full min-w-[48rem] text-left text-sm">
            <caption className="sr-only">Servers on all nodes</caption>
            <thead className="text-xs text-muted-foreground">
              <tr>
                {['Server', 'Node', 'Mode', 'Status', 'Game port', 'Updates', 'Actions'].map((h) => (
                  <th key={h} scope="col" className="px-4 py-3 font-medium">{h}</th>
                ))}
              </tr>
            </thead>
            <tbody>{servers.map((s) => <ServerRow key={`${s.node_id ?? ''}/${s.name}`} s={s} />)}</tbody>
          </table>
        </Card>
      )}
      <AddNodeDialog open={adding} onOpenChange={setAdding} />
      <ConfirmDialog
        open={removing !== null}
        onOpenChange={(o) => !o && setRemoving(null)}
        title={`Remove ${removing?.name ?? 'this node'}?`}
        description="It only disappears from this list. Nothing changes on that computer, and its servers keep running."
        confirmLabel="Remove node"
        destructive
        busy={remove.isPending}
        onConfirm={() => removing && remove.mutate(removing.id)}
      >
        {remove.isError && <ErrorNotice error={remove.error} />}
      </ConfirmDialog>
    </div>
  )
}
