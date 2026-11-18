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
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/settings/changeLicenceKey'

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
      <CardHeader><CardTitle>{t('settings.changeLicenceKey.licenceKey')}</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">{rich('settings.changeLicenceKey.nodeNodeChangeHere', { node: using ? <>{rich('settings.changeLicenceKey.serverUsesSavedLicence', { strong: <strong>{using.name}</strong>, last4: using.last4 })}</> : t('settings.changeLicenceKey.licenceKeyPartServers') })}</p>
        <Button onClick={() => { setDone(null); setOpen(true) }}>{t('settings.changeLicenceKey.changeLicenceKey')}</Button>
        {done && (
          <Alert tone={done.restart_needed ? 'warning' : 'success'} title={t('settings.changeLicenceKey.licenceKeyChangedNow', { last4: done.last4 })}>
            <p>{done.message}</p>
            {done.restart_needed && <div className="mt-2"><Button size="sm" variant="default" disabled={busy} onClick={restart}>{t('settings.changeLicenceKey.restartServerNow')}</Button></div>}
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
  const problem = choice === null ? t('settings.changeLicenceKey.pickOneSavedLicence') : choice === 'new' ? (key.trim().length < 8 ? t('settings.changeLicenceKey.pasteNewLicenceKey') : save && !saveName.trim() ? t('settings.changeLicenceKey.giveKeyNameTurn') : null) : null
  return (
    <Dialog open={open} onOpenChange={(o) => { if (!o) { onClose(); m.reset() } }}>
      <DialogContent title={t('settings.changeLicenceKey.changeLicenceKey')} description={t('settings.changeLicenceKey.chooseKeyNameShould', { name: server.name })}>
        <form className="space-y-4" onSubmit={(e) => { e.preventDefault(); if (!problem) m.mutate() }}>
          <ErrorNotice error={q.error} title={t('settings.changeLicenceKey.koraCannotReadSaved')} />
          {saved.length > 0 && (
            <fieldset className="space-y-2">
              <legend className="mb-1 text-sm text-muted-foreground">{t('settings.changeLicenceKey.savedKeys')}</legend>
              {saved.map((k) => (
                <Radio key={k.name} name="server-key" checked={choice === k.name} onChange={() => setPick(k.name)} label={k.name}
                  hint={t('settings.changeLicenceKey.keyEndingLastValue', { last4: k.last4, value: k.servers.includes(server.name) ? t('settings.changeLicenceKey.serverUsesNow') : '.' })} />
              ))}
              {canManage && <Radio name="server-key" checked={choice === 'new'} onChange={() => { setPick('new'); setSaveName(saveName || `Key ${saved.length + 1}`) }} label={t('settings.changeLicenceKey.newKey')} hint={t('settings.changeLicenceKey.pasteKeySavedYet')} />}
            </fieldset>
          )}
          {q.isSuccess && saved.length === 0 && !canManage && <Alert tone="info">{t('settings.changeLicenceKey.thereNoSavedLicence')}</Alert>}
          {choice === 'new' && (
            <div className="space-y-3">
              <SecretInput label={t('settings.changeLicenceKey.newLicenceKey')} value={key} onChange={setKey} hint={t('settings.changeLicenceKey.getOnePortalCfx')} />
              <Checkbox label={t('settings.changeLicenceKey.saveKeyComputer')} hint={t('settings.changeLicenceKey.pickNameNextTime')} checked={save} disabled={q.isError} onChange={(e) => { setSave(e.target.checked); if (e.target.checked && !saveName) setSaveName(t('settings.changeLicenceKey.defaultKeyName')) }} />
              {save && <Field label={t('settings.changeLicenceKey.nameKey')}><Input value={saveName} maxLength={40} onChange={(e) => setSaveName(e.target.value)} autoComplete="off" /></Field>}
            </div>
          )}
          {server.status === 'running'
            ? <p className="text-sm text-muted-foreground">{t('settings.changeLicenceKey.serverRunningOnlyReads')}</p>
            : <p className="text-sm text-muted-foreground">{t('settings.changeLicenceKey.serverStoppedNoRestart')}</p>}
          <ErrorNotice error={m.error} />
          <div className="flex justify-end gap-2">
            <Button type="button" onClick={onClose}>{t('settings.changeLicenceKey.cancel')}</Button>
            <Button type="submit" variant="default" disabled={m.isPending || !!problem} title={problem ?? undefined}>{t('settings.changeLicenceKey.changeLicenceKey')}</Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}
