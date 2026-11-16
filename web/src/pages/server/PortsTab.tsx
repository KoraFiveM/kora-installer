import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { Ports, ServerDetail } from '@/api/types'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { validatePorts } from '@/lib/utils'

export function PortsForm({ name, ports, running }: { name: string; ports: Ports; running: boolean }) {
  const qc = useQueryClient()
  const [game, setGame] = useState(String(ports.game))
  const [tx, setTx] = useState(String(ports.txadmin))
  const problem = validatePorts(game, tx)
  const m = useMutation({
    mutationFn: () => api.ports.set(name, { game: Number(game), txadmin: Number(tx) }),
    onSuccess: () => { void qc.invalidateQueries({ queryKey: ['server', name] }); void qc.invalidateQueries({ queryKey: ['servers'] }) },
  })
  const unchanged = Number(game) === ports.game && Number(tx) === ports.txadmin
  return (
    <form className="space-y-4" onSubmit={(e) => { e.preventDefault(); if (!problem && !running) m.mutate() }}>
      {running && <Alert tone="warning">Stop the server before changing its ports.</Alert>}
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label="Game port" hint="Players connect to this port."><Input inputMode="numeric" value={game} disabled={running} onChange={(e) => setGame(e.target.value)} /></Field>
        <Field label="txAdmin port" hint="The txAdmin web page uses this port."><Input inputMode="numeric" value={tx} disabled={running} onChange={(e) => setTx(e.target.value)} /></Field>
      </div>
      {problem && <p role="alert" className="text-sm text-destructive">{problem}</p>}
      <Button type="submit" variant="default" disabled={running || !!problem || unchanged || m.isPending}>Change ports</Button>
      {m.isSuccess && (
        <Alert tone="success" title="Ports changed">
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
      <CardHeader><CardTitle>Ports</CardTitle></CardHeader>
      <CardContent><PortsForm key={`${s.ports.game}-${s.ports.txadmin}`} name={s.name} ports={s.ports} running={s.status === 'running'} /></CardContent>
    </Card>
  )
}
