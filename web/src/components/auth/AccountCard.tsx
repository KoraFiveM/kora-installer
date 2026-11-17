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
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/auth'

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
      if ((await run({ step: 'passkey_finish', handle: begin.handle, label: label.trim() || t('auth.accountCard.myPasskey'), credential }))?.step === 'done') {
        setLabel('')
        void refresh()
      }
    } catch (e) {
      setErr(passkeyErrorText(e))
    }
  }

  return (
    <Card>
      <CardHeader><CardTitle>{t('auth.accountCard.yourSignin')}</CardTitle></CardHeader>
      <CardContent className="space-y-5 text-sm">
        {a.upgrade_suggested && (
          <Alert tone="warning" title={t('auth.accountCard.replaceTokenLink')}>{t('auth.accountCard.signedTokenSecretLink')}</Alert>
        )}
        {err && <Alert tone="danger">{err}</Alert>}

        <section className="space-y-2" aria-labelledby="passkeys-h">
          <h3 id="passkeys-h" className="font-semibold">{t('auth.accountCard.passkeys')}</h3>
          {a.passkeys.length === 0 && <p className="text-muted-foreground">{t('auth.accountCard.noneYet')}</p>}
          <ul className="divide-y divide-border rounded-md border border-border">
            {a.passkeys.map((p) => (
              <li key={p.id} className="flex items-center justify-between gap-3 px-3 py-2">
                <span>
                  <strong>{p.label}</strong> <span className="text-muted-foreground">{t('auth.accountCard.rpIdAddedWhen', { rp_id: p.rp_id, when: when(p.created), when2: when(p.last_used) })}</span>
                </span>
                <Button size="sm" variant="destructive" aria-label={t('auth.accountCard.removePasskeyLabel', { label: p.label })} onClick={() => setRemoving(p.id)}>{t('auth.accountCard.remove')}</Button>
              </li>
            ))}
          </ul>
          {a.passkey_problem ? (
            <Alert tone="info">{a.passkey_problem}</Alert>
          ) : passkeysSupported() ? (
            <form className="flex flex-wrap items-end gap-3" onSubmit={(e) => { e.preventDefault(); void addPasskey() }}>
              <div className="w-56"><Field label={t('auth.accountCard.nameDevice')}><Input value={label} onChange={(e) => setLabel(e.target.value)} placeholder={t('auth.accountCard.myPhone')} /></Field></div>
              <Button type="submit" disabled={change.isPending}>{t('auth.accountCard.addPasskey')}</Button>
            </form>
          ) : (
            <p className="text-muted-foreground">{t('auth.accountCard.browserCannotMakePasskeys')}</p>
          )}
        </section>

        <section className="space-y-2" aria-labelledby="codes-h">
          <h3 id="codes-h" className="font-semibold">{t('auth.accountCard.passwordOnetimeCodes')}</h3>
          {a.code ? (
            <p>{rich('auth.accountCard.setUpBadgeRecovery', { recovery_left: a.recovery_left, badge: (c) => <Badge tone="plain">{c}</Badge> })}</p>
          ) : (
            <p className="text-muted-foreground">{t('auth.accountCard.setUpUseThem')}</p>
          )}
          <div className="flex flex-wrap gap-3">
            {!a.code && !setup && <Button variant="secondary" onClick={async () => { const r = await run({ step: 'code_begin' }); if (r?.step === 'code_setup') setSetup(r) }}>{t('auth.accountCard.setUpOnetimeCodes')}</Button>}
            {a.code && <Button variant="secondary" onClick={async () => { const r = await run({ step: 'recovery_new' }); if (r?.step === 'recovery') { setCodes(r.codes); void refresh() } }}>{t('auth.accountCard.makeNewRecoveryCodes')}</Button>}
          </div>
          {setup && (
            <form className="space-y-3 rounded-md border border-border p-3" onSubmit={async (e) => { e.preventDefault(); const r = await run({ step: 'code_finish', password, code: code.trim() }); if (r?.step === 'recovery') { setCodes(r.codes); setSetup(null); setPassword(''); setCode(''); void refresh() } }}>
              <img className="block size-44 rounded-md bg-white p-2" alt={t('auth.accountCard.qrCodeAuthenticatorApp')} src={`data:image/svg+xml;utf8,${encodeURIComponent(setup.qr_svg)}`} />
              <CopyField value={setup.secret} label={t('auth.accountCard.setupKey')} />
              <Field label={t('auth.accountCard.choosePassword')} hint={t('auth.accountCard.leastCharacters')}><Input type="password" autoComplete="new-password" value={password} onChange={(e) => setPassword(e.target.value)} /></Field>
              <Field label={t('auth.accountCard.digitCodeAppShows')}><Input inputMode="numeric" autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} /></Field>
              <div className="flex gap-3"><Button type="submit" disabled={!password || !code.trim()}>{t('auth.accountCard.finish')}</Button><Button type="button" variant="ghost" onClick={() => setSetup(null)}>{t('auth.accountCard.cancel')}</Button></div>
            </form>
          )}
          {codes && (
            <Alert tone="success" title={t('auth.accountCard.recoveryCodes')}>
              <p className="mb-2">{t('auth.accountCard.keepThemSomewhereSafe')}</p>
              <ul className="grid grid-cols-2 gap-2 font-mono" aria-label={t('auth.accountCard.recoveryCodes2')}>{codes.map((c) => <li key={c} className="rounded bg-ink px-2 py-1">{c}</li>)}</ul>
              <div className="mt-2"><CopyField value={codes.join('\n')} label={t('auth.accountCard.allRecoveryCodes')} /></div>
              <Button className="mt-2" size="sm" variant="secondary" onClick={() => setCodes(null)}>{t('auth.accountCard.iSavedThem')}</Button>
            </Alert>
          )}
        </section>

        {a.legacy_token && (
          <section className="space-y-2" aria-labelledby="token-h">
            <h3 id="token-h" className="font-semibold">{t('auth.accountCard.oldTokenLink')}</h3>
            <p className="text-muted-foreground">{t('auth.accountCard.stillWorksScriptsSigning')}</p>
            <Button variant="destructive" disabled={!a.strong} onClick={() => setDropToken(true)}>{t('auth.accountCard.endMyTokenLink')}</Button>
          </section>
        )}

        <section className="space-y-2" aria-labelledby="devices-h">
          <h3 id="devices-h" className="font-semibold">{t('auth.accountCard.signedinDevices')}</h3>
          <ul className="divide-y divide-border rounded-md border border-border">
            {a.devices.map((d) => (
              <li key={d.handle} className="flex items-center justify-between gap-3 px-3 py-2">
                <span>
                  <strong>{d.device || t('auth.accountCard.aBrowser')}</strong> {d.current && <Badge tone="plain">{t('auth.accountCard.thisDevice')}</Badge>}
                  <span className="block text-xs text-muted-foreground">{t('auth.accountCard.valueSignedWhenLast', { value: d.address ? `${d.address}, ` : '', when: when(d.signed_in), when2: when(d.last_seen) })}</span>
                </span>
                {!d.current && <Button size="sm" variant="secondary" aria-label={t('auth.accountCard.signOutDevice', { device: d.device })} onClick={async () => { await run({ step: 'device_revoke', handle: d.handle }); void refresh() }}>{t('auth.accountCard.signOut')}</Button>}
              </li>
            ))}
          </ul>
          {a.devices.length > 1 && <Button variant="secondary" size="sm" onClick={async () => { await run({ step: 'devices_revoke_others' }); void refresh() }}>{t('auth.accountCard.signOutEverywhereElse')}</Button>}
        </section>

        <ErrorNotice error={q.error} />
        <ConfirmDialog
          open={removing !== null}
          onOpenChange={(o) => { if (!o) setRemoving(null) }}
          title={t('auth.accountCard.removePasskey')}
          description={t('auth.accountCard.deviceNoLongerSign')}
          confirmLabel={t('auth.accountCard.remove')}
          destructive
          busy={change.isPending}
          onConfirm={async () => { if (removing !== null) await run({ step: 'passkey_remove', id: removing }); setRemoving(null); void refresh() }}
        />
        <ConfirmDialog
          open={dropToken}
          onOpenChange={setDropToken}
          title={t('auth.accountCard.endOldTokenLink')}
          description={t('auth.accountCard.anythingStillUsesScript')}
          confirmLabel={t('auth.accountCard.endIt')}
          destructive
          busy={change.isPending}
          onConfirm={async () => { await run({ step: 'token_revoke' }); setDropToken(false); void refresh() }}
        />
      </CardContent>
    </Card>
  )
}
