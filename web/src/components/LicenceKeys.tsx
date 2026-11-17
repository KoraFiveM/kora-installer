import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Eye, EyeOff } from 'lucide-react'
import { api } from '@/api/client'
import type { Profile, SavedKey, SavedKeys } from '@/api/types'
import { useRole } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { t, msg, type Msg } from '@/i18n'
import '@/i18n/en/settings'

/** A key field that hides what is typed until the eye is pressed. */
export function SecretInput({ label, value, onChange, hint }: { label: string; value: string; onChange: (v: string) => void; hint?: string }) {
  const [show, setShow] = useState(false)
  return (
    <Field label={label} hint={hint}>
      <SecretBox value={value} onChange={onChange} show={show} toggle={() => setShow((x) => !x)} />
    </Field>
  )
}

function SecretBox({ value, onChange, show, toggle, id, ...rest }: { value: string; onChange: (v: string) => void; show: boolean; toggle: () => void; id?: string }) {
  return (
    <div className="flex gap-2">
      <Input id={id} {...rest} type={show ? 'text' : 'password'} value={value} onChange={(e) => onChange(e.target.value)} autoComplete="off" spellCheck={false} />
      <Button type="button" size="icon" aria-label={show ? t('settings.licenceKeys.hideLicenceKey') : t('settings.licenceKeys.showLicenceKey')} aria-pressed={show} onClick={toggle}>{show ? <EyeOff /> : <Eye />}</Button>
    </div>
  )
}

const when = (unix: number) => (unix > 0 ? new Date(unix * 1000).toLocaleDateString() : null)

type Dialogue = { kind: 'rename' | 'replace' | 'delete'; key: SavedKey } | null

/** Settings → Licence keys: the keys saved on this computer, by name. Admins manage them; others can only see the names. */
/** Where the keys are kept, and on this computer the switch for the system keychain. */
function KeychainLine({ data }: { data: SavedKeys }) {
  const qc = useQueryClient()
  const role = useRole()
  const [result, setResult] = useState<string[] | null>(null)
  const set = useMutation({
    mutationFn: (on: boolean) => api.keys.keychain(on),
    onSuccess: (r) => {
      setResult([`Moved ${r.moved} ${r.moved === 1 ? 'key' : 'keys'}.`, ...r.left.map((l) => t('settings.licenceKeys.couldMoveL', { l }))])
      void qc.invalidateQueries({ queryKey: ['keys'] })
    },
  })
  const kc = data.keychain
  return (
    <div className="space-y-2 text-sm">
      <p className="text-muted-foreground">{kc.line}</p>
      {!role.remote && role.can('admin') && (
        <div className="flex flex-wrap gap-3">
          {kc.mode === 'off' || !kc.available ? (
            <Button size="sm" variant="secondary" disabled={set.isPending || !kc.available && kc.mode !== 'off'} onClick={() => set.mutate(true)}>{t('settings.licenceKeys.useSystemKeychain')}</Button>
          ) : (
            <Button size="sm" variant="secondary" disabled={set.isPending} onClick={() => set.mutate(false)}>{t('settings.licenceKeys.keepKeysKorasPrivate')}</Button>
          )}
        </div>
      )}
      {result && result.map((r) => <Alert key={r} tone={r.startsWith('Could not') ? 'warning' : 'success'}>{r}</Alert>)}
      <ErrorNotice error={set.error} />
    </div>
  )
}

export function LicenceKeysCard() {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: ['keys'], queryFn: api.keys.list, retry: false })
  const [dialog, setDialog] = useState<Dialogue>(null)
  const [notes, setNotes] = useState<string[]>([])
  const [name, setName] = useState('')
  const [key, setKey] = useState('')
  const [show, setShow] = useState(false)
  const done = (msgs: string[] = []) => {
    setNotes(msgs)
    void qc.invalidateQueries({ queryKey: ['keys'] })
    // Saved answers may point at a key that was renamed or deleted.
    void qc.invalidateQueries({ queryKey: ['install-defaults'] })
  }
  const add = useMutation({
    mutationFn: () => api.keys.add(name, key),
    onSuccess: (r) => { setName(''); setKey(''); done([t('settings.licenceKeys.savedKeyEndingLast', { last4: r.key.last4, name: r.key.name })]) },
  })
  const keys = q.data?.keys ?? []
  const can = q.data?.can_manage ?? false
  return (
    <Card>
      <CardHeader><CardTitle>{t('settings.licenceKeys.licenceKeys')}</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">{t('settings.licenceKeys.keysSaveHerePicked')}</p>
        {q.data && <KeychainLine data={q.data} />}
        <ErrorNotice error={q.error} title={t('settings.licenceKeys.koraCannotReadSaved')} />
        {q.isSuccess && keys.length === 0 && <p className="text-sm">{t('settings.licenceKeys.noKeysSavedYet', { value: can ? t('settings.licenceKeys.addOneBelowTick') : t('settings.licenceKeys.askAdministratorAddOne') })}</p>}
        {keys.length > 0 && (
          <ul className="divide-y divide-border rounded-md border border-border" aria-label={t('settings.licenceKeys.savedLicenceKeys')}>
            {keys.map((k) => (
              <li key={k.name} className="flex flex-wrap items-center justify-between gap-3 p-3 text-sm">
                <div className="min-w-0 space-y-1">
                  <div className="font-medium">{k.name} <span className="font-normal text-muted-foreground">{t('settings.licenceKeys.keyEndingLast', { last4: k.last4 })}</span></div>
                  <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    {k.servers.length > 0 ? k.servers.map((s) => <Badge key={s} tone="neutral">{t('settings.licenceKeys.usedS', { s })}</Badge>) : <span>{t('settings.licenceKeys.usedAnyServerYet')}</span>}
                    {k.stopped_installs > 0 && <Badge tone="amber">{t('settings.licenceKeys.neededUnfinishedInstall')}</Badge>}
                    {when(k.added) && <span>{t('settings.licenceKeys.savedWhen', { when: when(k.added) ?? '' })}</span>}
                    <span>{t('settings.licenceKeys.keptKept', { kept: k.kept })}</span>
                  </div>
                  {k.problem && (
                    <Alert tone="warning" title={t('settings.licenceKeys.keyCannotReadFrom')}>
                      {k.problem}
                      {can && <div className="mt-2"><Button size="sm" aria-label={t('settings.licenceKeys.enterAgainName', { name: k.name })} onClick={() => { setDialog({ kind: 'replace', key: k }); setKey('') }}>{t('settings.licenceKeys.enterAgain')}</Button></div>}
                    </Alert>
                  )}
                </div>
                {can && (
                  <div className="flex gap-2">
                    <Button size="sm" aria-label={t('settings.licenceKeys.renameName', { name: k.name })} onClick={() => { setDialog({ kind: 'rename', key: k }); setName(k.name) }}>{t('settings.licenceKeys.rename')}</Button>
                    <Button size="sm" aria-label={t('settings.licenceKeys.replaceKeyName', { name: k.name })} onClick={() => { setDialog({ kind: 'replace', key: k }); setKey('') }}>{t('settings.licenceKeys.replaceKey')}</Button>
                    <Button size="sm" variant="destructive" aria-label={t('settings.licenceKeys.deleteName', { name: k.name })} onClick={() => setDialog({ kind: 'delete', key: k })}>{t('settings.licenceKeys.delete')}</Button>
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
        {notes.map((n) => <Alert key={n} tone="success">{n}</Alert>)}
        {q.isSuccess && !can && <p className="text-xs text-muted-foreground">{t('settings.licenceKeys.onlyAdministratorAddRename')}</p>}
        {can && (
          <form className="space-y-3 border-t border-border pt-4" onSubmit={(e) => { e.preventDefault(); add.mutate() }}>
            <p className="text-sm font-semibold">{t('settings.licenceKeys.addKey')}</p>
            <div className="grid gap-3 sm:grid-cols-2">
              <Field label={t('settings.licenceKeys.name')} hint={t('settings.licenceKeys.exampleMainTestServer')}><Input value={name} maxLength={40} onChange={(e) => setName(e.target.value)} autoComplete="off" /></Field>
              <Field label={t('settings.licenceKeys.licenceKey')} hint={t('settings.licenceKeys.getOnePortalCfx')}><SecretBox value={key} onChange={setKey} show={show} toggle={() => setShow((x) => !x)} /></Field>
            </div>
            <Button type="submit" variant="default" disabled={add.isPending || !name.trim() || !key.trim() || q.isError}>{t('settings.licenceKeys.saveKey')}</Button>
            <ErrorNotice error={add.error} />
          </form>
        )}
      </CardContent>
      <RenameDialog d={dialog} name={name} setName={setName} onClose={() => setDialog(null)} onDone={done} />
      <ReplaceDialog d={dialog} value={key} setValue={setKey} onClose={() => setDialog(null)} onDone={done} />
      <DeleteDialog d={dialog} onClose={() => setDialog(null)} onDone={done} />
    </Card>
  )
}

function RenameDialog({ d, name, setName, onClose, onDone }: { d: Dialogue; name: string; setName: (v: string) => void; onClose: () => void; onDone: (n: string[]) => void }) {
  const m = useMutation({
    mutationFn: () => api.keys.update(d!.key.name, { new_name: name }),
    onSuccess: (r) => { onClose(); onDone([t('settings.licenceKeys.renamedName', { name: r.key.name }), ...r.notes]) },
  })
  return (
    <Dialog open={d?.kind === 'rename'} onOpenChange={(o) => { if (!o) { onClose(); m.reset() } }}>
      <DialogContent title={t('settings.licenceKeys.renameKey')} description={t('settings.licenceKeys.serversWereInstalledAffected')}>
        <form className="space-y-4" onSubmit={(e) => { e.preventDefault(); m.mutate() }}>
          <Field label={t('settings.licenceKeys.newName')}><Input value={name} maxLength={40} onChange={(e) => setName(e.target.value)} autoComplete="off" /></Field>
          <ErrorNotice error={m.error} />
          <div className="flex justify-end gap-2">
            <Button type="button" onClick={onClose}>{t('settings.licenceKeys.cancel')}</Button>
            <Button type="submit" variant="default" disabled={m.isPending || !name.trim() || name.trim() === d?.key.name}>{t('settings.licenceKeys.rename')}</Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function ReplaceDialog({ d, value, setValue, onClose, onDone }: { d: Dialogue; value: string; setValue: (v: string) => void; onClose: () => void; onDone: (n: string[]) => void }) {
  const m = useMutation({
    mutationFn: () => api.keys.update(d!.key.name, { license_key: value }),
    onSuccess: (r) => { onClose(); setValue(''); onDone([t('settings.licenceKeys.nameNowHoldsKey', { name: r.key.name, last4: r.key.last4 }), ...r.notes]) },
  })
  return (
    <Dialog open={d?.kind === 'replace'} onOpenChange={(o) => { if (!o) { onClose(); m.reset() } }}>
      <DialogContent title={t('settings.licenceKeys.replaceKeyValue', { value: d?.key.name ?? '' })} description={t('settings.licenceKeys.putDifferentKeyUnder')}>
        <form className="space-y-4" onSubmit={(e) => { e.preventDefault(); m.mutate() }}>
          <SecretInput label={t('settings.licenceKeys.newLicenceKey')} value={value} onChange={setValue} />
          <ErrorNotice error={m.error} />
          <div className="flex justify-end gap-2">
            <Button type="button" onClick={onClose}>{t('settings.licenceKeys.cancel')}</Button>
            <Button type="submit" variant="default" disabled={m.isPending || !value.trim()}>{t('settings.licenceKeys.replaceKey')}</Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function DeleteDialog({ d, onClose, onDone }: { d: Dialogue; onClose: () => void; onDone: (n: string[]) => void }) {
  const m = useMutation({
    mutationFn: () => api.keys.remove(d!.key.name),
    onSuccess: (r) => { onClose(); onDone(r.notes) },
  })
  const k = d?.key
  return (
    <ConfirmDialog open={d?.kind === 'delete'} onOpenChange={(o) => { if (!o) { onClose(); m.reset() } }} title={t('settings.licenceKeys.deleteValue', { value: k?.name ?? '' })}
      description={t('settings.licenceKeys.forgetsKeyComputerAdd')} confirmLabel={t('settings.licenceKeys.deleteKey')} destructive busy={m.isPending} onConfirm={() => m.mutate()}>
      <div className="space-y-3 text-sm">
        <p>{t('settings.licenceKeys.serversWereInstalledKey')}</p>
        {k && k.stopped_installs > 0 && <Alert tone="warning">{t('settings.licenceKeys.unfinishedInstallUsesKey')}</Alert>}
        <ErrorNotice error={m.error} />
      </div>
    </ConfirmDialog>
  )
}

const LABELS: [keyof Profile, Msg][] = [['cfx_username', msg('settings.licenceKeys.cfxReName')], ['cfx_identifier', msg('settings.licenceKeys.cfxReAccount')], ['github_owner', msg('settings.licenceKeys.githubForksGo')]]

/** Settings → what Kora remembers between installs. Nothing here is secret. */
export function RememberedCard() {
  const qc = useQueryClient()
  const role = useRole()
  const q = useQuery({ queryKey: ['profile'], queryFn: api.profile.get })
  const forget = useMutation({
    mutationFn: (body: Partial<Profile>) => api.profile.set(body),
    onSuccess: () => { void qc.invalidateQueries({ queryKey: ['profile'] }); void qc.invalidateQueries({ queryKey: ['install-defaults'] }) },
  })
  const p = q.data
  const any = !!p && (p.cfx_username || p.cfx_identifier || p.github_owner)
  return (
    <Card>
      <CardHeader><CardTitle>{t('settings.licenceKeys.rememberedAnswers')}</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">{t('settings.licenceKeys.saveTypingKoraRemembers')}</p>
        {q.isSuccess && !any && <p className="text-sm">{t('settings.licenceKeys.nothingRememberedYet')}</p>}
        {p && any && (
          <dl className="divide-y divide-border text-sm">
            {LABELS.filter(([k]) => p[k]).map(([k, label]) => (
              <div key={k} className="flex items-center justify-between gap-3 py-2">
                <div><dt className="text-muted-foreground">{t(label)}</dt><dd>{p[k]}</dd></div>
                {role.can('admin') && <Button size="sm" aria-label={t('settings.licenceKeys.forgetLabel', { label: t(label) })} disabled={forget.isPending} onClick={() => forget.mutate(k === 'cfx_username' || k === 'cfx_identifier' ? { cfx_username: null, cfx_identifier: null } : { [k]: null })}>{t('settings.licenceKeys.forget')}</Button>}
              </div>
            ))}
          </dl>
        )}
        <ErrorNotice error={q.error ?? forget.error} />
      </CardContent>
    </Card>
  )
}
