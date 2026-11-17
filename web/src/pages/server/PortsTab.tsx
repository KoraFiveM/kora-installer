import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { Ports, ServerDetail } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { validatePorts } from '@/lib/utils'
import { t } from '@/i18n'
import '@/i18n/en/servers'

export function PortsForm({ name, ports, running }: { name: string; ports: Ports; running: boolean }) {
  const qc = useQueryClient()
  const [game, setGame] = useState(String(ports.game))
  const [tx, setTx] = useState(String(ports.txadmin))
  const problem = validatePorts(game, tx)
  const blocked = whyNot(useRole(), 'admin')
  const m = useMutation({
    mutationFn: () => api.ports.set(name, { game: Number(game), txadmin: Number(tx) }),
    onSuccess: () => { void qc.invalidateQueries({ queryKey: ['server', name] }); void qc.invalidateQueries({ queryKey: ['servers'] }) },
  })
  const unchanged = Number(game) === ports.game && Number(tx) === ports.txadmin
  return (
    <form className="space-y-4" onSubmit={(e) => { e.preventDefault(); if (!problem && !running && !blocked) m.mutate() }}>
      {running && <Alert tone="warning">{t('servers.portsTab.stopServerBeforeChanging')}</Alert>}
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label={t('servers.portsTab.gamePort')} hint={t('servers.portsTab.playersConnectPort')}><Input inputMode="numeric" value={game} disabled={running || !!blocked} onChange={(e) => setGame(e.target.value)} /></Field>
        <Field label={t('servers.portsTab.txadminPort')} hint={t('servers.portsTab.txadminWebPageUses')}><Input inputMode="numeric" value={tx} disabled={running || !!blocked} onChange={(e) => setTx(e.target.value)} /></Field>
      </div>
      {problem && <p role="alert" className="text-sm text-danger-text">{problem}</p>}
      <Button type="submit" variant="default" disabled={running || !!problem || unchanged || m.isPending || !!blocked} title={blocked ?? undefined}>{t('servers.portsTab.changePorts')}</Button>
      {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
      <p className="text-xs text-muted-foreground">{t('servers.portsTab.koraDoesChangeFirewall')}</p>
      {m.isSuccess && (
        <Alert tone="success" title={t('servers.portsTab.portsChanged')}>
          <ul className="list-disc pl-5">{m.data.notes.map((n, i) => <li key={i}>{n}</li>)}</ul>
        </Alert>
      )}
      <ErrorNotice error={m.error} />
    </form>
  )
}

export function PortsTab({ server: s }: { server: ServerDetail }) {
  return (
    <Card className="max-w-2xl">
      <CardHeader><CardTitle>{t('servers.portsTab.ports')}</CardTitle></CardHeader>
      <CardContent><PortsForm key={`${s.ports.game}-${s.ports.txadmin}`} name={s.name} ports={s.ports} running={s.status === 'running'} /></CardContent>
    </Card>
  )
}
