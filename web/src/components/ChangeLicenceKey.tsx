import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { ServerDetail, ServerKeyChanged } from '@/api/types'
import { useRole } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox, Radio } from '@/components/ui/checkbox'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { SecretInput } from '@/components/LicenceKeys'

/** Overview tab: which licence key this server uses, and a way to change it. */
export function LicenceKeyCard({ server, restart, busy }: { server: ServerDetail; restart: () => void; busy: boolean }) {
  const role = useRole()
  const q = useQuery({ queryKey: ['keys'], queryFn: api.keys.list, retry: false })
  const [open, setOpen] = useState(false)
  const [done, setDone] = useState<ServerKeyChanged | null>(null)
  if (!role.can('operator')) return null
  const using = q.data?.keys.find((k) => k.servers.includes(server.name))
  return (
    <Card>
      <CardHeader><CardTitle>Licence key</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">
          {using ? <>This server uses your saved key <strong>{using.name}</strong> (ending {using.last4}).</> : 'The licence key is part of this server’s own configuration.'}{' '}
          Change it here if you got a new key or want to move this server to another one.
        </p>
        <Button onClick={() => { setDone(null); setOpen(true) }}>Change licence key</Button>
        {done && (
          <Alert tone={done.restart_needed ? 'warning' : 'success'} title={`Licence key changed (now ending ${done.last4})`}>
            <p>{done.message}</p>
            {done.restart_needed && <div className="mt-2"><Button size="sm" variant="default" disabled={busy} onClick={restart}>Restart the server now</Button></div>}
          </Alert>
        )}
      </CardContent>
      <ChangeDialog server={server} open={open} onClose={() => setOpen(false)} onDone={(r) => { setOpen(false); setDone(r) }} />
    </Card>
  )
}

function ChangeDialog({ server, open, onClose, onDone }: { server: ServerDetail; open: boolean; onClose: () => void; onDone: (r: ServerKeyChanged) => void }) {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: ['keys'], queryFn: api.keys.list, retry: false, enabled: open })
  const saved = q.data?.keys ?? []
  const canManage = q.data?.can_manage ?? false
  const [pick, setPick] = useState<string | 'new' | null>(null)
  const [key, setKey] = useState('')
  const [save, setSave] = useState(true)
  const [saveName, setSaveName] = useState('')
  const choice = pick ?? (saved[0]?.name ?? (canManage ? 'new' : null))
  const m = useMutation({
    mutationFn: () => (choice === 'new'
      ? api.keys.setForServer(server.name, { license_key: key.trim(), ...(save && saveName.trim() ? { save_as: saveName.trim() } : {}) })
      : api.keys.setForServer(server.name, { name: choice as string })),
    onSuccess: (r) => {
      void qc.invalidateQueries({ queryKey: ['keys'] })
      setKey('')
      onDone(r)
    },
  })
  const problem = choice === null ? 'Pick one of the saved keys.' : choice === 'new' ? (key.trim().length < 8 ? 'Paste the new licence key.' : save && !saveName.trim() ? 'Give the key a name, or turn off "Save this key".' : null) : null
  return (
    <Dialog open={open} onOpenChange={(o) => { if (!o) { onClose(); m.reset() } }}>
      <DialogContent title="Change licence key" description={`Choose the key ${server.name} should use. Kora writes it into the server's own configuration.`}>
        <form className="space-y-4" onSubmit={(e) => { e.preventDefault(); if (!problem) m.mutate() }}>
          <ErrorNotice error={q.error} title="Kora cannot read your saved keys" />
          {saved.length > 0 && (
            <fieldset className="space-y-2">
              <legend className="mb-1 text-sm text-muted-foreground">Saved keys</legend>
              {saved.map((k) => (
                <Radio key={k.name} name="server-key" checked={choice === k.name} onChange={() => setPick(k.name)} label={k.name}
                  hint={`Key ending ${k.last4}${k.servers.includes(server.name) ? '. This server uses it now.' : '.'}`} />
              ))}
              {canManage && <Radio name="server-key" checked={choice === 'new'} onChange={() => { setPick('new'); setSaveName(saveName || `Key ${saved.length + 1}`) }} label="A new key" hint="Paste a key you have not saved yet." />}
            </fieldset>
          )}
          {q.isSuccess && saved.length === 0 && !canManage && <Alert tone="info">There are no saved keys to pick from. Ask an administrator to add one in Settings.</Alert>}
          {choice === 'new' && (
            <div className="space-y-3">
              <SecretInput label="New licence key" value={key} onChange={setKey} hint="Get one at portal.cfx.re, under Server keys." />
              <Checkbox label="Save this key on this computer" hint="So you can pick it by name next time." checked={save} disabled={q.isError} onChange={(e) => { setSave(e.target.checked); if (e.target.checked && !saveName) setSaveName('Main') }} />
              {save && <Field label="Name for this key"><Input value={saveName} maxLength={40} onChange={(e) => setSaveName(e.target.value)} autoComplete="off" /></Field>}
            </div>
          )}
          {server.status === 'running'
            ? <p className="text-sm text-muted-foreground">This server is running. It only reads its key when it starts, so you will be asked to restart it after the change.</p>
            : <p className="text-sm text-muted-foreground">This server is stopped, so no restart is needed: it uses the new key next time it starts.</p>}
          <ErrorNotice error={m.error} />
          <div className="flex justify-end gap-2">
            <Button type="button" onClick={onClose}>Cancel</Button>
            <Button type="submit" variant="default" disabled={m.isPending || !!problem} title={problem ?? undefined}>Change licence key</Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}
