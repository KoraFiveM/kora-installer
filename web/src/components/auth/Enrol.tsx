import { useEffect, useState } from 'react'
import { usePageTitle } from '@/hooks/useRouteAnnouncer'
import { api, ApiError } from '@/api/client'
import type { EnrolResponse } from '@/api/types'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { CopyField } from '@/components/CopyButton'
import { Field } from '@/components/Field'
import { KoraLogo } from '@/components/KoraLogo'
import { createPasskey, passkeyErrorText, passkeysSupported } from '@/lib/webauthn'

type Who = Extract<EnrolResponse, { step: 'who' }>
type CodeSetup = Extract<EnrolResponse, { step: 'code_setup' }>

function messageOf(e: unknown): string {
  return e instanceof ApiError ? e.message : passkeyErrorText(e)
}

/** The enrolment page (`/#enrol=<link>`): one person sets up their first passkey or one-time codes, then is signed in. */
export function Enrol({ link, onDone }: { link: string; onDone: () => void }) {
  usePageTitle('Set up your sign-in')
  const [who, setWho] = useState<Who | null>(null)
  const [gone, setGone] = useState<string | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [label, setLabel] = useState('')
  const [mode, setMode] = useState<'choose' | 'code'>('choose')
  const [setup, setSetup] = useState<CodeSetup | null>(null)
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [recovery, setRecovery] = useState<string[] | null>(null)

  useEffect(() => {
    api.auth
      .enrol({ step: 'check', link })
      .then((r) => r.step === 'who' && setWho(r))
      .catch((e) => setGone(messageOf(e)))
  }, [link])

  const canPasskey = !!who?.passkey && passkeysSupported()

  async function passkey() {
    setBusy(true)
    setErr(null)
    try {
      const begin = await api.auth.enrol({ step: 'passkey_begin', link })
      if (begin.step !== 'challenge') throw new Error('The panel did not start the set-up.')
      const credential = await createPasskey(begin.options)
      const done = await api.auth.enrol({ step: 'passkey_finish', link, handle: begin.handle, label: label.trim() || 'My passkey', credential })
      if (done.step === 'done') onDone()
    } catch (e) {
      setErr(messageOf(e))
    } finally {
      setBusy(false)
    }
  }

  async function startCode() {
    setBusy(true)
    setErr(null)
    try {
      const r = await api.auth.enrol({ step: 'code_begin', link })
      if (r.step === 'code_setup') {
        setSetup(r)
        setMode('code')
      }
    } catch (e) {
      setErr(messageOf(e))
    } finally {
      setBusy(false)
    }
  }

  async function finishCode() {
    setBusy(true)
    setErr(null)
    try {
      const r = await api.auth.enrol({ step: 'code_finish', link, password, code: code.trim() })
      if (r.step === 'done') setRecovery(r.recovery_codes)
    } catch (e) {
      setErr(messageOf(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <main className="mx-auto flex min-h-screen max-w-lg flex-col justify-center gap-6 p-6">
      <KoraLogo className="h-9 w-auto text-foreground" />
      <Card>
        <CardHeader>
          <CardTitle as="h1" className="text-xl">{who ? `Welcome, ${who.user}` : 'Set up your sign-in'}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          {gone && <Alert tone="danger">{gone}</Alert>}
          {!who && !gone && <p className="text-sm text-muted-foreground">Checking your link…</p>}
          {who && recovery === null && (
            <>
              <p className="text-sm text-muted-foreground">
                You were invited to Kora Panel as <strong>{who.role}</strong>. This link works once. Set up how you will sign in from now on, so nobody needs to pass a secret link around again.
              </p>
              {err && <Alert tone="danger">{err}</Alert>}
              {mode === 'choose' && (
                <div className="space-y-4">
                  {canPasskey ? (
                    <div className="space-y-3 rounded-md border border-border p-3">
                      <h2 className="text-sm font-semibold">Passkey (recommended)</h2>
                      <p className="text-sm text-muted-foreground">Your phone, laptop or security key keeps a private key and asks for your fingerprint, face or PIN. Nothing to remember or type.</p>
                      <Field label="Name this device" hint="So you can tell your passkeys apart later, for example Work laptop.">
                        <Input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="My laptop" />
                      </Field>
                      <Button disabled={busy} onClick={() => void passkey()}>Set up a passkey</Button>
                    </div>
                  ) : (
                    <Alert tone="info">{who.passkey_problem ?? 'This browser cannot make a passkey here.'} You can use a password and one-time codes instead.</Alert>
                  )}
                  <div className="space-y-3 rounded-md border border-border p-3">
                    <h2 className="text-sm font-semibold">Password and one-time codes</h2>
                    <p className="text-sm text-muted-foreground">Works everywhere. You need an authenticator app (any app that scans a QR code: your phone's, 1Password, Aegis, and so on).</p>
                    <Button variant={canPasskey ? 'secondary' : 'default'} disabled={busy} onClick={() => void startCode()}>Set up one-time codes</Button>
                  </div>
                </div>
              )}
              {mode === 'code' && setup && (
                <form className="space-y-4" onSubmit={(e) => { e.preventDefault(); void finishCode() }}>
                  <ol className="list-decimal space-y-3 pl-5 text-sm">
                    <li>
                      Scan this with your authenticator app.
                      <img className="mt-2 block size-52 rounded-md bg-white p-2" alt="QR code for your authenticator app" src={`data:image/svg+xml;utf8,${encodeURIComponent(setup.qr_svg)}`} />
                      <p className="mt-2 text-xs text-muted-foreground">Cannot scan? Type this key into the app instead:</p>
                      <CopyField value={setup.secret} label="Setup key" />
                    </li>
                    <li>
                      <Field label="Choose a password" hint="At least 12 characters. A few unrelated words make a strong, easy-to-remember one.">
                        <Input type="password" autoComplete="new-password" value={password} onChange={(e) => setPassword(e.target.value)} />
                      </Field>
                    </li>
                    <li>
                      <Field label="Enter the 6-digit code the app shows">
                        <Input inputMode="numeric" autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} />
                      </Field>
                    </li>
                  </ol>
                  <div className="flex gap-3">
                    <Button type="submit" disabled={busy || !password || !code.trim()}>Finish set-up</Button>
                    <Button type="button" variant="ghost" onClick={() => { setMode('choose'); setErr(null) }}>Back</Button>
                  </div>
                </form>
              )}
            </>
          )}
          {recovery !== null && (
            <div className="space-y-4">
              <Alert tone="success" title="You are set up and signed in">
                {recovery.length > 0 ? 'Keep these recovery codes somewhere safe, apart from your phone. Each works once, if you lose your authenticator. They are shown only now.' : 'You can close this page, or carry on to the panel.'}
              </Alert>
              {recovery.length > 0 && (
                <ul className="grid grid-cols-2 gap-2 font-mono text-sm" aria-label="Recovery codes">
                  {recovery.map((c) => (
                    <li key={c} className="rounded bg-ink px-2 py-1">{c}</li>
                  ))}
                </ul>
              )}
              {recovery.length > 0 && <CopyField value={recovery.join('\n')} label="All recovery codes" />}
              <Button onClick={onDone}>I have saved them, carry on</Button>
            </div>
          )}
        </CardContent>
      </Card>
    </main>
  )
}
