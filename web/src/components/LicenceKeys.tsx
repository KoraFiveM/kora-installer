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
      <Button type="button" size="icon" aria-label={show ? 'Hide the licence key' : 'Show the licence key'} aria-pressed={show} onClick={toggle}>{show ? <EyeOff /> : <Eye />}</Button>
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
      setResult([`Moved ${r.moved} ${r.moved === 1 ? 'key' : 'keys'}.`, ...r.left.map((l) => `Could not move: ${l}`)])
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
            <Button size="sm" variant="secondary" disabled={set.isPending || !kc.available && kc.mode !== 'off'} onClick={() => set.mutate(true)}>Use the system keychain</Button>
          ) : (
            <Button size="sm" variant="secondary" disabled={set.isPending} onClick={() => set.mutate(false)}>Keep keys in Kora&apos;s private file instead</Button>
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
    onSuccess: (r) => { setName(''); setKey(''); done([`Saved the key ending ${r.key.last4} as "${r.key.name}".`]) },
  })
  const keys = q.data?.keys ?? []
  const can = q.data?.can_manage ?? false
  return (
    <Card>
      <CardHeader><CardTitle>Licence keys</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">
          Keys you save here can be picked by name when you install a server or change a server&apos;s key, so you never paste them twice.
          They stay on this computer, and Kora only ever shows the last 4 characters.
        </p>
        {q.data && <KeychainLine data={q.data} />}
        <ErrorNotice error={q.error} title="Kora cannot read your saved licence keys" />
        {q.isSuccess && keys.length === 0 && <p className="text-sm">No keys saved yet.{can ? ' Add one below, or tick "Save this key" when you install a server.' : ' Ask an administrator to add one.'}</p>}
        {keys.length > 0 && (
          <ul className="divide-y divide-border rounded-md border border-border" aria-label="Saved licence keys">
            {keys.map((k) => (
              <li key={k.name} className="flex flex-wrap items-center justify-between gap-3 p-3 text-sm">
                <div className="min-w-0 space-y-1">
                  <div className="font-medium">{k.name} <span className="font-normal text-muted-foreground">key ending {k.last4}</span></div>
                  <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    {k.servers.length > 0 ? k.servers.map((s) => <Badge key={s} tone="neutral">Used by {s}</Badge>) : <span>Not used by any server yet</span>}
                    {k.stopped_installs > 0 && <Badge tone="amber">Needed by an unfinished install</Badge>}
                    {when(k.added) && <span>Saved {when(k.added)}</span>}
                    <span>Kept in {k.kept}</span>
                  </div>
                  {k.problem && (
                    <Alert tone="warning" title="This key cannot be read from here">
                      {k.problem}
                      {can && <div className="mt-2"><Button size="sm" aria-label={`Enter it again: ${k.name}`} onClick={() => { setDialog({ kind: 'replace', key: k }); setKey('') }}>Enter it again</Button></div>}
                    </Alert>
                  )}
                </div>
                {can && (
                  <div className="flex gap-2">
                    <Button size="sm" aria-label={`Rename ${k.name}`} onClick={() => { setDialog({ kind: 'rename', key: k }); setName(k.name) }}>Rename</Button>
                    <Button size="sm" aria-label={`Replace key of ${k.name}`} onClick={() => { setDialog({ kind: 'replace', key: k }); setKey('') }}>Replace key</Button>
                    <Button size="sm" variant="destructive" aria-label={`Delete ${k.name}`} onClick={() => setDialog({ kind: 'delete', key: k })}>Delete</Button>
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
        {notes.map((n) => <Alert key={n} tone="success">{n}</Alert>)}
        {q.isSuccess && !can && <p className="text-xs text-muted-foreground">Only an administrator can add, rename, replace or delete keys. You can pick any of these when you install a server.</p>}
        {can && (
          <form className="space-y-3 border-t border-border pt-4" onSubmit={(e) => { e.preventDefault(); add.mutate() }}>
            <p className="text-sm font-semibold">Add a key</p>
            <div className="grid gap-3 sm:grid-cols-2">
              <Field label="Name" hint='For example "Main" or "Test server".'><Input value={name} maxLength={40} onChange={(e) => setName(e.target.value)} autoComplete="off" /></Field>
              <Field label="Licence key" hint="Get one at portal.cfx.re, under Server keys."><SecretBox value={key} onChange={setKey} show={show} toggle={() => setShow((x) => !x)} /></Field>
            </div>
            <Button type="submit" variant="default" disabled={add.isPending || !name.trim() || !key.trim() || q.isError}>Save key</Button>
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
    onSuccess: (r) => { onClose(); onDone([`Renamed to "${r.key.name}".`, ...r.notes]) },
  })
  return (
    <Dialog open={d?.kind === 'rename'} onOpenChange={(o) => { if (!o) { onClose(); m.reset() } }}>
      <DialogContent title="Rename this key" description="Servers that were installed with it are not affected. Unfinished installs follow the new name.">
        <form className="space-y-4" onSubmit={(e) => { e.preventDefault(); m.mutate() }}>
          <Field label="New name"><Input value={name} maxLength={40} onChange={(e) => setName(e.target.value)} autoComplete="off" /></Field>
          <ErrorNotice error={m.error} />
          <div className="flex justify-end gap-2">
            <Button type="button" onClick={onClose}>Cancel</Button>
            <Button type="submit" variant="default" disabled={m.isPending || !name.trim() || name.trim() === d?.key.name}>Rename</Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function ReplaceDialog({ d, value, setValue, onClose, onDone }: { d: Dialogue; value: string; setValue: (v: string) => void; onClose: () => void; onDone: (n: string[]) => void }) {
  const m = useMutation({
    mutationFn: () => api.keys.update(d!.key.name, { license_key: value }),
    onSuccess: (r) => { onClose(); setValue(''); onDone([`"${r.key.name}" now holds the key ending ${r.key.last4}.`, ...r.notes]) },
  })
  return (
    <Dialog open={d?.kind === 'replace'} onOpenChange={(o) => { if (!o) { onClose(); m.reset() } }}>
      <DialogContent title={`Replace key of "${d?.key.name ?? ''}"`} description="Put a different key under this name. Servers already installed keep the key they have; use Change licence key on a server to switch it.">
        <form className="space-y-4" onSubmit={(e) => { e.preventDefault(); m.mutate() }}>
          <SecretInput label="New licence key" value={value} onChange={setValue} />
          <ErrorNotice error={m.error} />
          <div className="flex justify-end gap-2">
            <Button type="button" onClick={onClose}>Cancel</Button>
            <Button type="submit" variant="default" disabled={m.isPending || !value.trim()}>Replace key</Button>
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
    <ConfirmDialog open={d?.kind === 'delete'} onOpenChange={(o) => { if (!o) { onClose(); m.reset() } }} title={`Delete "${k?.name ?? ''}"?`}
      description="This forgets the key on this computer. You can add it again from the Cfx.re portal." confirmLabel="Delete key" destructive busy={m.isPending} onConfirm={() => m.mutate()}>
      <div className="space-y-3 text-sm">
        <p>Servers that were installed with this key keep working: they have their own copy.</p>
        {k && k.stopped_installs > 0 && <Alert tone="warning">An unfinished install uses this key. It will need another key picked before it can carry on.</Alert>}
        <ErrorNotice error={m.error} />
      </div>
    </ConfirmDialog>
  )
}

const LABELS: [keyof Profile, string][] = [['cfx_username', 'Cfx.re name'], ['cfx_identifier', 'Cfx.re account'], ['github_owner', 'GitHub forks go to']]

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
      <CardHeader><CardTitle>Remembered answers</CardTitle></CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">To save you typing, Kora remembers your Cfx.re name and your GitHub owner after you first use them, and fills them in on the install page. You can change them there at any time. Nothing here is secret.</p>
        {q.isSuccess && !any && <p className="text-sm">Nothing is remembered yet.</p>}
        {p && any && (
          <dl className="divide-y divide-border text-sm">
            {LABELS.filter(([k]) => p[k]).map(([k, label]) => (
              <div key={k} className="flex items-center justify-between gap-3 py-2">
                <div><dt className="text-muted-foreground">{label}</dt><dd>{p[k]}</dd></div>
                {role.can('admin') && <Button size="sm" aria-label={`Forget ${label}`} disabled={forget.isPending} onClick={() => forget.mutate(k === 'cfx_username' || k === 'cfx_identifier' ? { cfx_username: null, cfx_identifier: null } : { [k]: null })}>Forget</Button>}
              </div>
            ))}
          </dl>
        )}
        <ErrorNotice error={q.error ?? forget.error} />
      </CardContent>
    </Card>
  )
}
