import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react'
import { api, ApiError, setReauthHandler } from '@/api/client'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Field } from '@/components/Field'
import { getPasskey, passkeyErrorText, passkeysSupported } from '@/lib/webauthn'
import { t } from '@/i18n'
import '@/i18n/en/auth'

const Ctx = createContext<() => Promise<boolean>>(() => Promise.resolve(false))
/** Ask the person to confirm it is them (passkey or one-time code). Resolves true when they did. */
export const useReauth = () => useContext(Ctx)

/**
 * The fresh check before the most sensitive actions (deleting a server, uninstalling, console input, changing how you sign in):
 * when the panel answers `remote.reauth_required`, this asks for the passkey or a code and the request is sent again.
 */
export function ReauthProvider({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false)
  const waiting = useRef<((ok: boolean) => void)[]>([])
  const [err, setErr] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [useCode, setUseCode] = useState(!passkeysSupported())
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')

  const settle = useCallback((ok: boolean) => {
    setOpen(false)
    setErr(null)
    setPassword('')
    setCode('')
    const w = waiting.current
    waiting.current = []
    w.forEach((f) => f(ok))
  }, [])

  const ask = useCallback(
    () =>
      new Promise<boolean>((resolve) => {
        waiting.current.push(resolve)
        setOpen(true)
      }),
    [],
  )
  useEffect(() => {
    setReauthHandler(ask)
    return () => setReauthHandler(null)
  }, [ask])

  async function withPasskey() {
    setBusy(true)
    setErr(null)
    try {
      const begin = await api.auth.reauth({ step: 'passkey_begin' })
      if (begin.step !== 'challenge') throw new Error(t('auth.reauthProvider.panelDidStartCheck'))
      const credential = await getPasskey(begin.options)
      await api.auth.reauth({ step: 'passkey_finish', handle: begin.handle, credential })
      settle(true)
    } catch (e) {
      setErr(e instanceof ApiError ? e.message : passkeyErrorText(e))
    } finally {
      setBusy(false)
    }
  }

  async function withCode() {
    setBusy(true)
    setErr(null)
    try {
      await api.auth.reauth({ step: 'code', password, code: code.trim() })
      settle(true)
    } catch (e) {
      setErr(e instanceof ApiError ? e.message : passkeyErrorText(e))
      setPassword('')
      setCode('')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Ctx.Provider value={ask}>
      {children}
      <Dialog open={open} onOpenChange={(o) => { if (!o) settle(false) }}>
        <DialogContent title={t('auth.reauthProvider.confirmItIsYou')} description={t('auth.reauthProvider.sensitiveActionUsePasskey')}>
          {err && <Alert tone="danger">{err}</Alert>}
          {!useCode ? (
            <div className="flex flex-wrap gap-3">
              <Button disabled={busy} onClick={() => void withPasskey()}>{t('auth.reauthProvider.useMyPasskey')}</Button>
              <Button variant="ghost" onClick={() => setUseCode(true)}>{t('auth.reauthProvider.useOnetimeCodeInstead')}</Button>
            </div>
          ) : (
            <form className="space-y-3" onSubmit={(e) => { e.preventDefault(); void withCode() }}>
              <Field label={t('auth.reauthProvider.password')}><Input type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} /></Field>
              <Field label={t('auth.reauthProvider.onetimeCode')}><Input autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} /></Field>
              <div className="flex gap-3">
                <Button type="submit" disabled={busy || !password || !code.trim()}>{t('auth.reauthProvider.confirm')}</Button>
                {passkeysSupported() && <Button type="button" variant="ghost" onClick={() => setUseCode(false)}>{t('auth.reauthProvider.useMyPasskeyInstead')}</Button>}
              </div>
            </form>
          )}
        </DialogContent>
      </Dialog>
    </Ctx.Provider>
  )
}
