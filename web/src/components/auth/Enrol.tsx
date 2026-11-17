import { useEffect, useState } from 'react'
import { LanguagePicker } from '@/components/LanguagePicker'
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
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/auth/enrol'

type Who = Extract<EnrolResponse, { step: 'who' }>
type CodeSetup = Extract<EnrolResponse, { step: 'code_setup' }>

function messageOf(e: unknown): string {
  return e instanceof ApiError ? e.message : passkeyErrorText(e)
}

/** The enrolment page (`/#enrol=<link>`): one person sets up their first passkey or one-time codes, then is signed in. */
export function Enrol({ link, onDone }: { link: string; onDone: () => void }) {
  usePageTitle(t('auth.enrol.setUpSignin'))
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
      if (begin.step !== 'challenge') throw new Error(t('auth.enrol.panelDidStartSetup'))
      const credential = await createPasskey(begin.options)
      const done = await api.auth.enrol({ step: 'passkey_finish', link, handle: begin.handle, label: label.trim() || t('auth.enrol.myPasskey'), credential })
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
      <div className="flex items-center justify-between gap-4">
        <KoraLogo className="h-9 w-auto text-foreground" />
        <LanguagePicker compact />
      </div>
      <Card>
        <CardHeader>
          <CardTitle as="h1" className="text-xl">{who ? t('auth.enrol.welcomeUser', { user: who.user }) : t('auth.enrol.setUpSignin')}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          {gone && <Alert tone="danger">{gone}</Alert>}
          {!who && !gone && <p className="text-sm text-muted-foreground">{t('auth.enrol.checkingLink')}</p>}
          {who && recovery === null && (
            <>
              <p className="text-sm text-muted-foreground">{rich('auth.enrol.wereInvitedKoraPanel', { strong: <strong>{who.role}</strong> })}</p>
              {err && <Alert tone="danger">{err}</Alert>}
              {mode === 'choose' && (
                <div className="space-y-4">
                  {canPasskey ? (
                    <div className="space-y-3 rounded-md border border-border p-3">
                      <h2 className="text-sm font-semibold">{t('auth.enrol.passkeyRecommended')}</h2>
                      <p className="text-sm text-muted-foreground">{t('auth.enrol.phoneLaptopSecurityKey')}</p>
                      <Field label={t('auth.enrol.nameDevice')} hint={t('auth.enrol.tellPasskeysApartLater')}>
                        <Input value={label} onChange={(e) => setLabel(e.target.value)} placeholder={t('auth.enrol.myLaptop')} />
                      </Field>
                      <Button disabled={busy} onClick={() => void passkey()}>{t('auth.enrol.setUpPasskey')}</Button>
                    </div>
                  ) : (
                    <Alert tone="info">{t('auth.enrol.valueUsePasswordOnetime', { value: who.passkey_problem ?? t('auth.enrol.browserCannotMakePasskey') })}</Alert>
                  )}
                  <div className="space-y-3 rounded-md border border-border p-3">
                    <h2 className="text-sm font-semibold">{t('auth.enrol.passwordOnetimeCodes')}</h2>
                    <p className="text-sm text-muted-foreground">{t('auth.enrol.worksEverywhereNeedAuthenticat')}</p>
                    <Button variant={canPasskey ? 'secondary' : 'default'} disabled={busy} onClick={() => void startCode()}>{t('auth.enrol.setUpOnetimeCodes')}</Button>
                  </div>
                </div>
              )}
              {mode === 'code' && setup && (
                <form className="space-y-4" onSubmit={(e) => { e.preventDefault(); void finishCode() }}>
                  <ol className="list-decimal space-y-3 pl-5 text-sm">
                    <li>{rich('auth.enrol.scanAuthenticatorAppImg', { img: <img className="mt-2 block size-52 rounded-md bg-white p-2" alt={t('auth.enrol.qrCodeAuthenticatorApp')} src={`data:image/svg+xml;utf8,${encodeURIComponent(setup.qr_svg)}`} />, copyField: <CopyField value={setup.secret} label={t('auth.enrol.setupKey')} />, p: (c) => <p className="mt-2 text-xs text-muted-foreground">{c}</p> })}</li>
                    <li>
                      <Field label={t('auth.enrol.choosePassword')} hint={t('auth.enrol.leastCharactersFewUnrelated')}>
                        <Input type="password" autoComplete="new-password" value={password} onChange={(e) => setPassword(e.target.value)} />
                      </Field>
                    </li>
                    <li>
                      <Field label={t('auth.enrol.enterDigitCodeApp')}>
                        <Input inputMode="numeric" autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} />
                      </Field>
                    </li>
                  </ol>
                  <div className="flex gap-3">
                    <Button type="submit" disabled={busy || !password || !code.trim()}>{t('auth.enrol.finishSetup')}</Button>
                    <Button type="button" variant="ghost" onClick={() => { setMode('choose'); setErr(null) }}>{t('auth.enrol.back')}</Button>
                  </div>
                </form>
              )}
            </>
          )}
          {recovery !== null && (
            <div className="space-y-4">
              <Alert tone="success" title={t('auth.enrol.setUpSigned')}>
                {recovery.length > 0 ? t('auth.enrol.keepTheseRecoveryCodes') : t('auth.enrol.closePageCarryPanel')}
              </Alert>
              {recovery.length > 0 && (
                <ul className="grid grid-cols-2 gap-2 font-mono text-sm" aria-label={t('auth.enrol.recoveryCodes')}>
                  {recovery.map((c) => (
                    <li key={c} className="rounded bg-ink px-2 py-1">{c}</li>
                  ))}
                </ul>
              )}
              {recovery.length > 0 && <CopyField value={recovery.join('\n')} label={t('auth.enrol.allRecoveryCodes')} />}
              <Button onClick={onDone}>{t('auth.enrol.iSavedThemCarry')}</Button>
            </div>
          )}
        </CardContent>
      </Card>
    </main>
  )
}
