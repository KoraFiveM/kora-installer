import { useState } from 'react'
import type { ApiError } from '@/api/client'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Field } from '@/components/Field'
import { Alert } from '@/components/ui/alert'

/** Opened from another computer (not 127.0.0.1 / localhost)? Then the `kora panel` link cannot work here. */
function openedRemotely(): boolean {
  const h = window.location.hostname
  return !(h === '127.0.0.1' || h === 'localhost' || h === '[::1]' || h === '::1')
}

/** Shown when the API answers 401: not signed in, or the session ran out. */
export function SignedOut({ error, onToken, onIdentity }: { error: ApiError | null; onToken: (token: string) => Promise<ApiError | null>; onIdentity?: () => Promise<ApiError | null> }) {
  const [token, setToken] = useState('')
  const [err, setErr] = useState<ApiError | null>(error)
  const [busy, setBusy] = useState(false)
  const remote = openedRemotely()
  return (
    <main className="mx-auto flex min-h-screen max-w-lg flex-col justify-center gap-6 p-6">
      <div className="font-heading text-2xl font-bold">Kora</div>
      <Card>
        <CardHeader>
          <CardTitle className="text-xl">Session expired, or not signed in</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          {remote ? (
            <p className="text-sm text-muted-foreground">
              Sign in with your Tailscale account if an administrator has added it (they can, on the computer that runs Kora, with <code className="rounded bg-ink px-1.5 py-0.5 font-mono text-xs">kora panel tailscale allow you@example.com</code>). Or paste the user token you were given.
            </p>
          ) : (
            <p className="text-sm text-muted-foreground">
              To keep your servers safe, this page needs a fresh sign-in link. Run <code className="rounded bg-ink px-1.5 py-0.5 font-mono text-xs">kora panel</code> on this computer and open the link it prints (each link works once). If you have a sign-in code or link instead, paste it below.
            </p>
          )}
          {err && <Alert tone="danger">{err.message}</Alert>}
          {/* Only from other devices: Tailscale cannot vouch for this computer, so it is not offered here. */}
          {remote && onIdentity && (
            <Button variant="default" disabled={busy} onClick={async () => { setBusy(true); setErr(await onIdentity()); setBusy(false) }}>Sign in with Tailscale</Button>
          )}
          <form
            className="space-y-3"
            onSubmit={async (e) => {
              e.preventDefault()
              if (!token.trim()) return
              setBusy(true)
              setErr(await onToken(token))
              setBusy(false)
            }}
          >
            <Field label={remote ? 'User token' : 'Sign-in code or link'}>
              <Input type="password" autoComplete="off" value={token} onChange={(e) => setToken(e.target.value)} />
            </Field>
            <Button type="submit" variant={remote ? 'secondary' : 'default'} disabled={busy || !token.trim()}>
              Sign in
            </Button>
          </form>
        </CardContent>
      </Card>
    </main>
  )
}
