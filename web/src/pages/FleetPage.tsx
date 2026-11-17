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
import { t, msg, type Msg } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/servers/fleetPage'

const ING: Record<ControlAction, Msg> = { start: msg('servers.fleetPage.starting'), stop: msg('servers.fleetPage.stopping'), restart: msg('servers.fleetPage.restarting') }
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
    if (j.state === 'failed') push('danger', `${label}: ${j.error?.message ?? t('servers.fleetPage.actionDidFinish')}`)
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
      push('danger', `${label}: ${toNotice(e)?.message ?? t('servers.fleetPage.actionDidStart')}`)
    } finally {
      setStarting(false)
    }
  }
  const busy = starting || view.phase === 'running'
  return { run, busy, pending: busy && action ? t(ING[action]) : undefined }
}

function Actions({ s, busy, run }: { s: FleetServer; busy: boolean; run: (a: ControlAction) => void }) {
  const [confirm, setConfirm] = useState(false)
  if (s.status === 'missing') return <span className="text-xs text-muted-foreground">{t('servers.fleetPage.folderMissing')}</span>
  if (s.actions.length === 0) return <span className="text-xs text-muted-foreground">{t('servers.fleetPage.updateNodeControlFrom')}</span>
  const can = (a: ControlAction) => s.actions.includes(a)
  return (
    <div className="flex gap-2">
      {s.status === 'stopped' && can('start') && (
        <Button size="sm" disabled={busy} onClick={() => run('start')} aria-label={t('servers.fleetPage.startNameNodeName', { name: s.name, node_name: s.node_name })}>{rich('servers.fleetPage.playNodeStart', { play: <Play /> })}</Button>
      )}
      {s.status === 'running' && can('stop') && (
        <Button size="sm" disabled={busy} onClick={() => run('stop')} aria-label={t('servers.fleetPage.stopNameNodeName', { name: s.name, node_name: s.node_name })}>{rich('servers.fleetPage.squareNodeStop', { square: <Square /> })}</Button>
      )}
      {s.status === 'running' && can('restart') && (
        <Button size="sm" disabled={busy} onClick={() => setConfirm(true)} aria-label={t('servers.fleetPage.restartNameNodeName', { name: s.name, node_name: s.node_name })}>{rich('servers.fleetPage.rotatecwNodeRestart', { rotateCw: <RotateCw /> })}</Button>
      )}
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title={t('servers.fleetPage.restartName', { name: s.name })}
        description={t('servers.fleetPage.everyonePlayingNameNode', { name: s.name, node_name: s.node_name })}
        confirmLabel={t('servers.fleetPage.restartServer')}
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
      push(n.insecure ? 'warning' : 'success', n.insecure ? t('servers.fleetPage.addedButInsecure', { name: n.name, warning: n.warning ?? t('servers.fleetPage.addressNotPrivate') }) : t('servers.fleetPage.nodeAdded', { name: n.name }))
      setName('')
      setUrl('')
      setToken('')
      onOpenChange(false)
    },
  })
  return (
    <Dialog open={open} onOpenChange={(o) => { if (!o) add.reset(); onOpenChange(o) }}>
      <DialogContent title={t('servers.fleetPage.addNode')} description={t('servers.fleetPage.nodeAnotherComputerRunning')}>
        <form className="space-y-4" onSubmit={(e) => { e.preventDefault(); add.mutate() }}>
          <Field label={t('servers.fleetPage.name')}><Input value={name} onChange={(e) => setName(e.target.value)} placeholder={t('servers.fleetPage.garageBox')} autoComplete="off" /></Field>
          <Field label={t('servers.fleetPage.address')} hint={t('servers.fleetPage.exampleHttpUseHttps')}><Input value={url} onChange={(e) => setUrl(e.target.value)} placeholder={t('servers.fleetPage.http')} autoComplete="off" /></Field>
          <Field label={t('servers.fleetPage.token')} hint={t('servers.fleetPage.savedComputerOnlyNever')}><Input type="password" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" /></Field>
          {add.isError && <ErrorNotice error={add.error} />}
          <div className="flex justify-end gap-2">
            <Button type="button" onClick={() => onOpenChange(false)}>{t('servers.fleetPage.cancel')}</Button>
            <Button type="submit" variant="default" disabled={add.isPending || !name.trim() || !url.trim() || !token.trim()}>{add.isPending ? t('servers.fleetPage.checking') : t('servers.fleetPage.addNode2')}</Button>
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
          {n.reachable ? <Badge tone="success" dot>{t('servers.fleetPage.online')}</Badge> : <Badge tone="danger" dot>{t('servers.fleetPage.notReachable')}</Badge>}
          {n.version && <Badge tone="plain">Kora {n.version}</Badge>}
          {n.reachable && !n.can_control && <Badge tone="orange">{t('servers.fleetPage.viewOnlyUpdateControl')}</Badge>}
        </div>
        <div className="truncate font-mono text-xs text-muted-foreground">{n.url}</div>
        {n.server_count !== null && <div className="text-xs text-muted-foreground">{t('servers.fleetPage.serversCount', { count: n.server_count })}</div>}
        {n.last_error && <div className="text-xs text-danger-text">{n.last_error}</div>}
        {n.warning && <div className="text-xs text-orange">{n.warning}</div>}
      </div>
      <Button size="sm" onClick={onRemove} aria-label={t('servers.fleetPage.removeName', { name: n.name })}>{rich('servers.fleetPage.trashNodeRemove', { trash2: <Trash2 /> })}</Button>
    </li>
  )
}

export default function FleetPage() {
  const { remote } = useRole()
  if (remote) {
    return (
      <div className="space-y-6">
        <h1 className="font-heading text-2xl font-bold">{t('servers.fleetPage.fleet')}</h1>
        <Alert tone="info">{t('servers.fleetPage.fleetOnlyOpenedComputer')}</Alert>
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
      push('success', t('servers.fleetPage.nodeRemoved', { name: removing?.name ?? t('servers.fleetPage.theNode') }))
      setRemoving(null)
      void qc.invalidateQueries({ queryKey: ['fleet'] })
    },
  })
  const nodes = q.data?.nodes ?? []
  const servers = q.data?.servers ?? []
  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="font-heading text-2xl font-bold">{t('servers.fleetPage.fleet')}</h1>
        <div className="flex gap-2">
          <Button onClick={() => void q.refetch()} disabled={q.isFetching}>{rich('servers.fleetPage.refreshcwNodeRefresh', { refreshCw: <RefreshCw className={q.isFetching ? 'animate-spin' : ''} /> })}</Button>
          <Button onClick={() => setAdding(true)}>{rich('servers.fleetPage.plusNodeAddNode', { plus: <Plus /> })}</Button>
        </div>
      </div>
      {q.isError && <ErrorNotice error={q.error} />}
      {q.data?.partial && <Alert tone="warning" title={t('servers.fleetPage.someNodesAnswering')}>{t('servers.fleetPage.listBelowShowsWhat')}</Alert>}
      {q.isLoading && <Skeleton className="h-40" />}
      {q.isSuccess && (
        <Card>
          <CardHeader><CardTitle>{t('servers.fleetPage.nodes')}</CardTitle></CardHeader>
          <CardContent>
            {nodes.length === 0 ? (
              <p className="text-sm text-muted-foreground">{t('servers.fleetPage.noOtherPanelsYet')}</p>
            ) : (
              <ul className="space-y-3">{nodes.map((n) => <NodeCard key={n.id} n={n} onRemove={() => setRemoving(n)} />)}</ul>
            )}
          </CardContent>
        </Card>
      )}
      {servers.length > 0 && (
        <Card className="overflow-x-auto">
          <table className="w-full min-w-[48rem] text-left text-sm">
            <caption className="sr-only">{t('servers.fleetPage.serversAllNodes')}</caption>
            <thead className="text-xs text-muted-foreground">
              <tr>
                {['Server', 'Node', 'Mode', 'Status', t('servers.fleetPage.gamePort'), 'Updates', 'Actions'].map((h) => (
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
        title={t('servers.fleetPage.removeValue', { name: removing?.name ?? 'this node' })}
        description={t('servers.fleetPage.onlyDisappearsFromList')}
        confirmLabel={t('servers.fleetPage.removeNode')}
        destructive
        busy={remove.isPending}
        onConfirm={() => removing && remove.mutate(removing.id)}
      >
        {remove.isError && <ErrorNotice error={remove.error} />}
      </ConfirmDialog>
    </div>
  )
}
