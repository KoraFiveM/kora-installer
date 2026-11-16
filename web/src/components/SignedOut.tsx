import { useState } from 'react'
import type { ApiError } from '@/api/client'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Field } from '@/components/Field'
import { Alert } from '@/components/ui/alert'

/** Shown when the API answers 401: not signed in, or the session ran out. */
export function SignedOut({ error, onToken, onIdentity }: { error: ApiError | null; onToken: (token: string) => Promise<ApiError | null>; onIdentity?: () => Promise<ApiError | null> }) {
  const [token, setToken] = useState('')
  const [err, setErr] = useState<ApiError | null>(error)
  const [busy, setBusy] = useState(false)
  return (
    <main className="mx-auto flex min-h-screen max-w-lg flex-col justify-center gap-6 p-6">
      <div className="font-heading text-2xl font-bold">Kora</div>
      <Card>
        <CardHeader>
          <CardTitle className="text-xl">Session expired, or not signed in</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <p className="text-sm text-muted-foreground">
            To keep your servers safe, this page needs a fresh sign-in link. Run <code className="rounded bg-ink px-1.5 py-0.5 font-mono text-xs">kora panel</code> on the computer that runs Kora and open the link it prints. If you have a sign-in code instead, paste it below.
          </p>
          {err && <Alert tone="danger">{err.message}</Alert>}
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
            <Field label="Sign-in code">
              <Input type="password" autoComplete="off" value={token} onChange={(e) => setToken(e.target.value)} />
            </Field>
            <Button type="submit" variant="default" disabled={busy || !token.trim()}>
              Sign in
            </Button>
          </form>
          {onIdentity && (
            <div className="space-y-2 border-t border-border pt-4">
              <p className="text-sm text-muted-foreground">On a private network with Tailscale? If an administrator has added your account, you can sign in without a code.</p>
              <Button variant="secondary" disabled={busy} onClick={async () => { setBusy(true); setErr(await onIdentity()); setBusy(false) }}>Sign in with Tailscale</Button>
            </div>
          )}
        </CardContent>
      </Card>
    </main>
  )
}
