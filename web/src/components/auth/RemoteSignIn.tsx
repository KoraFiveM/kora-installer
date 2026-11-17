import { useState } from 'react'
import { api, ApiError } from '@/api/client'
import type { AuthOptions } from '@/api/types'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field } from '@/components/Field'
import { getPasskey, passkeyErrorText, passkeysSupported } from '@/lib/webauthn'

type Method = 'passkey' | 'code'

/** What a failed step says: an ApiError's message, or the browser's own passkey error. */
function messageOf(e: unknown): string {
  return e instanceof ApiError ? e.message : passkeyErrorText(e)
}

/**
 * Signing in from another computer: a passkey first (type your name, touch your device), a password with a one-time code as the
 * alternative where passkeys cannot work (an address with no name, an old browser). `onSignedIn` runs once the session exists.
 */
export function RemoteSignIn({ options, onSignedIn }: { options: AuthOptions; onSignedIn: (notice: string | null) => void }) {
  const canPasskey = options.passkey && passkeysSupported()
  const [method, setMethod] = useState<Method>(canPasskey ? 'passkey' : 'code')
  const [user, setUser] = useState('')
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)

  const why = !options.passkey ? options.passkey_problem : !passkeysSupported() ? 'This browser cannot use passkeys here. Sign in with a one-time code instead.' : null

  async function withPasskey() {
    setBusy(true)
    setErr(null)
    try {
      const begin = await api.auth.login({ step: 'passkey_begin', user: user.trim() })
      if (begin.step !== 'challenge') throw new Error('The panel did not start the sign-in.')
      const credential = await getPasskey(begin.options)
      const done = await api.auth.login({ step: 'passkey_finish', handle: begin.handle, credential })
      if (done.step === 'signed_in') onSignedIn(done.notice)
    } catch (e) {
      setErr(messageOf(e))
    } finally {
      setBusy(false)
    }
  }

  async function withCode() {
    setBusy(true)
    setErr(null)
    try {
      const done = await api.auth.login({ step: 'code', user: user.trim(), password, code: code.trim() })
      if (done.step === 'signed_in') onSignedIn(done.notice)
    } catch (e) {
      setErr(messageOf(e))
      setPassword('')
      setCode('')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="space-y-4">
      {why && method === 'passkey' && <Alert tone="info">{why}</Alert>}
      {err && <Alert tone="danger">{err}</Alert>}
      <form
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault()
          if (!user.trim()) return
          void (method === 'passkey' ? withPasskey() : withCode())
        }}
      >
        <Field label="Your name" hint="The name the person who runs Kora gave you.">
          <Input autoComplete="username webauthn" value={user} onChange={(e) => setUser(e.target.value)} />
        </Field>
        {method === 'code' && (
          <>
            <Field label="Password">
              <Input type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} />
            </Field>
            <Field label="One-time code" hint="The 6 digits from your authenticator app. Lost your phone? Use one of your recovery codes here.">
              <Input inputMode="text" autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} />
            </Field>
          </>
        )}
        <div className="flex flex-wrap items-center gap-3">
          {method === 'passkey' ? (
            <Button type="submit" disabled={busy || !user.trim() || !canPasskey}>
              Sign in with a passkey
            </Button>
          ) : (
            <Button type="submit" disabled={busy || !user.trim() || !password || !code.trim()}>
              Sign in with a code
            </Button>
          )}
          {options.code && canPasskey && method === 'passkey' && (
            <Button type="button" variant="ghost" onClick={() => { setMethod('code'); setErr(null) }}>
              Use a one-time code instead
            </Button>
          )}
          {method === 'code' && canPasskey && (
            <Button type="button" variant="ghost" onClick={() => { setMethod('passkey'); setErr(null) }}>
              Use a passkey instead
            </Button>
          )}
        </div>
      </form>
      {method === 'passkey' && !canPasskey && options.code && (
        <Button type="button" variant="secondary" onClick={() => setMethod('code')}>
          Use a one-time code
        </Button>
      )}
    </div>
  )
}
