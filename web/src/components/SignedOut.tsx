import { useEffect, useState } from 'react'
import { api, type ApiError } from '@/api/client'
import type { AuthOptions } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Field } from '@/components/Field'
import { Alert } from '@/components/ui/alert'
import { KoraLogo } from '@/components/KoraLogo'
import { usePageTitle } from '@/hooks/useRouteAnnouncer'
import { RemoteSignIn } from '@/components/auth/RemoteSignIn'

/** Opened from another computer (not 127.0.0.1 / localhost)? Used until the panel has said (`/meta`), and if it cannot. */
function openedRemotely(): boolean {
  const h = window.location.hostname
  return !(h === '127.0.0.1' || h === 'localhost' || h === '[::1]' || h === '::1')
}

/** Shown when the API answers 401: not signed in, or the session ran out. */
export function SignedOut({
  error,
  onToken,
  onIdentity,
  onSignedIn,
}: {
  error: ApiError | null
  onToken: (token: string) => Promise<ApiError | null>
  onIdentity?: () => Promise<ApiError | null>
  /** A passkey or one-time-code sign-in finished (the session exists). */
  onSignedIn?: (notice: string | null) => void
}) {
  usePageTitle('Sign in')
  const [token, setToken] = useState('')
  const [err, setErr] = useState<ApiError | null>(error)
  const [busy, setBusy] = useState(false)
  const [remote, setRemote] = useState(openedRemotely())
  const [options, setOptions] = useState<AuthOptions | null>(null)
  const [showToken, setShowToken] = useState(false)
  useEffect(() => {
    let live = true
    // Both answers are public: the panel says whether this visit is from another computer and what it offers.
    api.meta().then((m) => live && setRemote(m.remote)).catch(() => undefined)
    api.auth.options().then((o) => live && setOptions(o)).catch(() => undefined)
    return () => { live = false }
  }, [])

  const tokenForm = (
    <form
      className="space-y-3"
      onSubmit={async (e) => {
        e.preventDefault()
        if (!token.trim()) return
        // An enrolment link pasted here: open it as the page it is.
        const at = token.indexOf('#enrol=')
        if (at >= 0) {
          window.location.hash = token.slice(at)
          window.location.reload()
          return
        }
        setBusy(true)
        setErr(await onToken(token))
        setBusy(false)
      }}
    >
      <Field
        label={remote ? 'Old token link' : 'Sign-in link'}
        hint={remote
          ? 'Only if you still have a token from before. It still works for now, and the panel will help you replace it with a passkey.'
          : 'Paste the whole link the command printed. It starts with http://.'}
      >
        <Input type="password" autoComplete="off" value={token} onChange={(e) => setToken(e.target.value)} />
      </Field>
      <Button type="submit" variant={remote ? 'secondary' : 'default'} disabled={busy || !token.trim()}>
        Sign in
      </Button>
    </form>
  )

  return (
    <main className="mx-auto flex min-h-screen max-w-lg flex-col justify-center gap-6 p-6">
      <KoraLogo className="h-9 w-auto text-foreground" />
      <Card>
        <CardHeader>
          <CardTitle as="h1" className="text-xl">Sign in to Kora</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          {remote ? (
            <>
              <p className="text-sm text-muted-foreground">
                Sign in with the passkey you set up, or with a one-time code. New here? Open the one-time link the person who runs Kora gave you.
              </p>
              {err && <Alert tone="danger">{err.message}</Alert>}
              {options?.tailscale && onIdentity && (
                <Button variant="default" disabled={busy} onClick={async () => { setBusy(true); setErr(await onIdentity()); setBusy(false) }}>Sign in with Tailscale</Button>
              )}
              {!options && onIdentity && (
                <Button variant="default" disabled={busy} onClick={async () => { setBusy(true); setErr(await onIdentity()); setBusy(false) }}>Sign in with Tailscale</Button>
              )}
              {options && onSignedIn && <RemoteSignIn options={options} onSignedIn={onSignedIn} />}
              {options?.strong_required && <p className="text-xs text-muted-foreground">An administrator requires a passkey or one-time codes here, so the old token link is not accepted.</p>}
              {(options?.token ?? true) && (
                <div className="space-y-2 border-t border-border pt-3">
                  {showToken ? tokenForm : <Button variant="ghost" size="sm" onClick={() => setShowToken(true)}>I have an old token link</Button>}
                </div>
              )}
            </>
          ) : (
            <>
              <p className="text-sm text-muted-foreground">
                To keep your servers safe, this page needs a sign-in link. Run <code className="rounded bg-ink px-1.5 py-0.5 font-mono text-xs">kora panel --print-link</code> on this computer to get a new sign-in link. Open that link, or paste it below. If the command says the background service is not installed, run <code className="rounded bg-ink px-1.5 py-0.5 font-mono text-xs">kora panel</code> instead and use the link it prints.
              </p>
              {err && <Alert tone="danger">{err.message}</Alert>}
              {tokenForm}
            </>
          )}
        </CardContent>
      </Card>
    </main>
  )
}
