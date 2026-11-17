import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api, ApiError } from '@/api/client'
import type { AccountChange, AccountChanged } from '@/api/types'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { CopyField } from '@/components/CopyButton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { timeAgo } from '@/lib/ago'
import { createPasskey, passkeyErrorText, passkeysSupported } from '@/lib/webauthn'

const when = (unix: number) => (unix ? timeAgo(unix * 1000, Date.now()) : 'never')

/**
 * "Your sign-in" for someone signed in from another computer with a passkey, one-time codes or the old token: passkeys,
 * one-time codes and recovery codes, the old token link, and the signed-in devices. Changing how you sign in asks for a fresh
 * check first (the app-wide dialog), once you have something strong to check with.
 */
export function AccountCard() {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: ['auth-account'], queryFn: api.auth.account, retry: false })
  const [label, setLabel] = useState('')
  const [err, setErr] = useState<string | null>(null)
  const [codes, setCodes] = useState<string[] | null>(null)
  const [setup, setSetup] = useState<Extract<AccountChanged, { step: 'code_setup' }> | null>(null)
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [removing, setRemoving] = useState<number | null>(null)
  const [dropToken, setDropToken] = useState(false)
  const refresh = () => qc.invalidateQueries({ queryKey: ['auth-account'] })
  const change = useMutation({ mutationFn: (b: AccountChange) => api.auth.change(b) })
  const run = async (b: AccountChange) => {
    setErr(null)
    try {
      return await change.mutateAsync(b)
    } catch (e) {
      // (A fresh check, when the panel asks for one, is handled for every request by the app: see ReauthProvider.)
      setErr(e instanceof ApiError ? e.message : passkeyErrorText(e))
      return null
    }
  }
  const a = q.data
  if (q.isError || !a) return null

  async function addPasskey() {
    const begin = await run({ step: 'passkey_begin' })
    if (begin?.step !== 'challenge') return
    try {
      const credential = await createPasskey(begin.options)
      if ((await run({ step: 'passkey_finish', handle: begin.handle, label: label.trim() || 'My passkey', credential }))?.step === 'done') {
        setLabel('')
        void refresh()
      }
    } catch (e) {
      setErr(passkeyErrorText(e))
    }
  }

  return (
    <Card>
      <CardHeader><CardTitle>Your sign-in</CardTitle></CardHeader>
      <CardContent className="space-y-5 text-sm">
        {a.upgrade_suggested && (
          <Alert tone="warning" title="Replace your token link">
            You signed in with a token: a secret link that works for whoever holds it. Set up a passkey or one-time codes below. After that the token link stops working for signing in.
          </Alert>
        )}
        {err && <Alert tone="danger">{err}</Alert>}

        <section className="space-y-2" aria-labelledby="passkeys-h">
          <h3 id="passkeys-h" className="font-semibold">Passkeys</h3>
          {a.passkeys.length === 0 && <p className="text-muted-foreground">None yet.</p>}
          <ul className="divide-y divide-border rounded-md border border-border">
            {a.passkeys.map((p) => (
              <li key={p.id} className="flex items-center justify-between gap-3 px-3 py-2">
                <span>
                  <strong>{p.label}</strong> <span className="text-muted-foreground">for {p.rp_id}, added {when(p.created)}, last used {when(p.last_used)}</span>
                </span>
                <Button size="sm" variant="destructive" aria-label={`Remove passkey ${p.label}`} onClick={() => setRemoving(p.id)}>Remove</Button>
              </li>
            ))}
          </ul>
          {a.passkey_problem ? (
            <Alert tone="info">{a.passkey_problem}</Alert>
          ) : passkeysSupported() ? (
            <form className="flex flex-wrap items-end gap-3" onSubmit={(e) => { e.preventDefault(); void addPasskey() }}>
              <div className="w-56"><Field label="Name this device"><Input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="My phone" /></Field></div>
              <Button type="submit" disabled={change.isPending}>Add a passkey</Button>
            </form>
          ) : (
            <p className="text-muted-foreground">This browser cannot make passkeys here.</p>
          )}
        </section>

        <section className="space-y-2" aria-labelledby="codes-h">
          <h3 id="codes-h" className="font-semibold">Password and one-time codes</h3>
          {a.code ? (
            <p>Set up. <Badge tone="plain">{a.recovery_left} recovery codes left</Badge></p>
          ) : (
            <p className="text-muted-foreground">Not set up. Use them where a passkey cannot work (an address with no name, an old browser).</p>
          )}
          <div className="flex flex-wrap gap-3">
            {!a.code && !setup && <Button variant="secondary" onClick={async () => { const r = await run({ step: 'code_begin' }); if (r?.step === 'code_setup') setSetup(r) }}>Set up one-time codes</Button>}
            {a.code && <Button variant="secondary" onClick={async () => { const r = await run({ step: 'recovery_new' }); if (r?.step === 'recovery') { setCodes(r.codes); void refresh() } }}>Make new recovery codes</Button>}
          </div>
          {setup && (
            <form className="space-y-3 rounded-md border border-border p-3" onSubmit={async (e) => { e.preventDefault(); const r = await run({ step: 'code_finish', password, code: code.trim() }); if (r?.step === 'recovery') { setCodes(r.codes); setSetup(null); setPassword(''); setCode(''); void refresh() } }}>
              <img className="block size-44 rounded-md bg-white p-2" alt="QR code for your authenticator app" src={`data:image/svg+xml;utf8,${encodeURIComponent(setup.qr_svg)}`} />
              <CopyField value={setup.secret} label="Setup key" />
              <Field label="Choose a password" hint="At least 12 characters."><Input type="password" autoComplete="new-password" value={password} onChange={(e) => setPassword(e.target.value)} /></Field>
              <Field label="The 6-digit code the app shows"><Input inputMode="numeric" autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} /></Field>
              <div className="flex gap-3"><Button type="submit" disabled={!password || !code.trim()}>Finish</Button><Button type="button" variant="ghost" onClick={() => setSetup(null)}>Cancel</Button></div>
            </form>
          )}
          {codes && (
            <Alert tone="success" title="Your recovery codes">
              <p className="mb-2">Keep them somewhere safe. Each works once. They are shown only now.</p>
              <ul className="grid grid-cols-2 gap-2 font-mono" aria-label="Recovery codes">{codes.map((c) => <li key={c} className="rounded bg-ink px-2 py-1">{c}</li>)}</ul>
              <div className="mt-2"><CopyField value={codes.join('\n')} label="All recovery codes" /></div>
              <Button className="mt-2" size="sm" variant="secondary" onClick={() => setCodes(null)}>I have saved them</Button>
            </Alert>
          )}
        </section>

        {a.legacy_token && (
          <section className="space-y-2" aria-labelledby="token-h">
            <h3 id="token-h" className="font-semibold">Old token link</h3>
            <p className="text-muted-foreground">Still works for scripts, and for signing in until you set up something stronger. Once you have, end it so a leaked link is useless.</p>
            <Button variant="destructive" disabled={!a.strong} onClick={() => setDropToken(true)}>End my token link</Button>
          </section>
        )}

        <section className="space-y-2" aria-labelledby="devices-h">
          <h3 id="devices-h" className="font-semibold">Signed-in devices</h3>
          <ul className="divide-y divide-border rounded-md border border-border">
            {a.devices.map((d) => (
              <li key={d.handle} className="flex items-center justify-between gap-3 px-3 py-2">
                <span>
                  <strong>{d.device || 'A browser'}</strong> {d.current && <Badge tone="plain">this device</Badge>}
                  <span className="block text-xs text-muted-foreground">{d.address ? `${d.address}, ` : ''}signed in {when(d.signed_in)}, last seen {when(d.last_seen)}</span>
                </span>
                {!d.current && <Button size="sm" variant="secondary" aria-label={`Sign out ${d.device}`} onClick={async () => { await run({ step: 'device_revoke', handle: d.handle }); void refresh() }}>Sign out</Button>}
              </li>
            ))}
          </ul>
          {a.devices.length > 1 && <Button variant="secondary" size="sm" onClick={async () => { await run({ step: 'devices_revoke_others' }); void refresh() }}>Sign out everywhere else</Button>}
        </section>

        <ErrorNotice error={q.error} />
        <ConfirmDialog
          open={removing !== null}
          onOpenChange={(o) => { if (!o) setRemoving(null) }}
          title="Remove this passkey?"
          description="That device can no longer sign in, and your other signed-in devices are signed out."
          confirmLabel="Remove"
          destructive
          busy={change.isPending}
          onConfirm={async () => { if (removing !== null) await run({ step: 'passkey_remove', id: removing }); setRemoving(null); void refresh() }}
        />
        <ConfirmDialog
          open={dropToken}
          onOpenChange={setDropToken}
          title="End your old token link?"
          description="Anything that still uses it (a script) stops working. Your passkey or codes keep working."
          confirmLabel="End it"
          destructive
          busy={change.isPending}
          onConfirm={async () => { await run({ step: 'token_revoke' }); setDropToken(false); void refresh() }}
        />
      </CardContent>
    </Card>
  )
}
