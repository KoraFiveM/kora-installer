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
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/auth'

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
  usePageTitle(t('auth.signedOut.signIn'))
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
        label={remote ? t('auth.signedOut.oldTokenLink') : t('auth.signedOut.signinLink')}
        hint={remote
          ? t('auth.signedOut.onlyStillTokenFrom')
          : t('auth.signedOut.pasteWholeLinkCommand')}
      >
        <Input type="password" autoComplete="off" value={token} onChange={(e) => setToken(e.target.value)} />
      </Field>
      <Button type="submit" variant={remote ? 'secondary' : 'default'} disabled={busy || !token.trim()}>{t('auth.signedOut.signIn2')}</Button>
    </form>
  )

  return (
    <main className="mx-auto flex min-h-screen max-w-lg flex-col justify-center gap-6 p-6">
      <KoraLogo className="h-9 w-auto text-foreground" />
      <Card>
        <CardHeader>
          <CardTitle as="h1" className="text-xl">{t('auth.signedOut.signKora')}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          {remote ? (
            <>
              <p className="text-sm text-muted-foreground">{t('auth.signedOut.signPasskeySetUp')}</p>
              {err && <Alert tone="danger">{err.message}</Alert>}
              {options?.tailscale && onIdentity && (
                <Button variant="default" disabled={busy} onClick={async () => { setBusy(true); setErr(await onIdentity()); setBusy(false) }}>{t('auth.signedOut.signTailscale')}</Button>
              )}
              {!options && onIdentity && (
                <Button variant="default" disabled={busy} onClick={async () => { setBusy(true); setErr(await onIdentity()); setBusy(false) }}>{t('auth.signedOut.signTailscale')}</Button>
              )}
              {options && onSignedIn && <RemoteSignIn options={options} onSignedIn={onSignedIn} />}
              {options?.strong_required && <p className="text-xs text-muted-foreground">{t('auth.signedOut.administratorRequiresPasskeyOn')}</p>}
              {(options?.token ?? true) && (
                <div className="space-y-2 border-t border-border pt-3">
                  {showToken ? tokenForm : <Button variant="ghost" size="sm" onClick={() => setShowToken(true)}>{t('auth.signedOut.iOldTokenLink')}</Button>}
                </div>
              )}
            </>
          ) : (
            <>
              <p className="text-sm text-muted-foreground">{rich('auth.signedOut.keepServersSafePage', { code: (c) => <code className="rounded bg-ink px-1.5 py-0.5 font-mono text-xs">{c}</code>, code2: (c) => <code className="rounded bg-ink px-1.5 py-0.5 font-mono text-xs">{c}</code> })}</p>
              {err && <Alert tone="danger">{err.message}</Alert>}
              {tokenForm}
            </>
          )}
        </CardContent>
      </Card>
    </main>
  )
}
