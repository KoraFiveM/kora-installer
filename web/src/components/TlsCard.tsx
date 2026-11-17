import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import { useRole } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { CopyField } from '@/components/CopyButton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { t } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/settings'

const SOURCE: Record<string, string> = {
  provided: 'a certificate you provided',
  tailscale: 'a Tailscale certificate',
  'self-signed': 'a self-signed certificate made by Kora',
}

const TRUST_STEPS = [
  ['Windows (Edge, Chrome)', 'Double-click the saved file, choose Install Certificate, Current User, then place it in Trusted Root Certification Authorities. Restart the browser.'],
  ['macOS (Safari, Chrome)', 'Double-click the file to add it to Keychain Access, open it there and set "When using this certificate" to Always Trust.'],
  ['Linux (Chrome, Chromium) and Firefox', 'In the browser settings, Certificates, Authorities: import the file and tick "Trust this CA to identify websites".'],
  ['iPhone, iPad, Android', 'Send the file to the device and open it, then switch it on under Certificate Trust Settings (iPhone) or install it as a CA certificate (Android).'],
]

function when(unix: number): string {
  return new Date(unix * 1000).toLocaleDateString('en-GB', { day: 'numeric', month: 'long', year: 'numeric' })
}

/** Settings > HTTPS: which certificate the panel uses, until when, its fingerprint, renew now. */
export function TlsCard() {
  const qc = useQueryClient()
  const role = useRole()
  const q = useQuery({ queryKey: ['tls'], queryFn: api.tls.status, retry: false })
  const renew = useMutation({ mutationFn: api.tls.renew, onSuccess: (d) => qc.setQueryData(['tls'], d) })
  const [ca, setCa] = useState<string | null>(null)
  const get = useMutation({ mutationFn: api.tls.ca, onSuccess: (c) => setCa(c.pem) })
  const s = q.data
  if (!s) return null
  const soon = s.days_left !== null && s.days_left <= 14
  return (
    <Card>
      <CardHeader><CardTitle>HTTPS</CardTitle></CardHeader>
      <CardContent className="space-y-4 text-sm">
        {s.active ? (
          <>
            <p>{rich('settings.tlsCard.panelUsesValueBadge', { value: SOURCE[s.source ?? ''] ?? 'a certificate', badge: <Badge tone={soon ? 'amber' : 'plain'}>{s.days_left !== null && s.days_left >= 0 ? `${s.days_left} days left` : 'expired'}</Badge> })}</p>
            <dl className="grid gap-x-4 gap-y-1 sm:grid-cols-[10rem_1fr]">
              <dt className="text-muted-foreground">{t('settings.tlsCard.validUntil')}</dt><dd>{when(s.not_after)}</dd>
              <dt className="text-muted-foreground">{t('settings.tlsCard.names')}</dt><dd>{s.names.join(', ')}</dd>
              <dt className="text-muted-foreground">{t('settings.tlsCard.fingerprintSha')}</dt><dd><code className="break-all font-mono text-xs">{s.fingerprint}</code></dd>
              {s.ca_fingerprint && (<><dt className="text-muted-foreground">{t('settings.tlsCard.localAuthority')}</dt><dd><code className="break-all font-mono text-xs">{s.ca_fingerprint}</code></dd></>)}
            </dl>
            <p className="text-xs text-muted-foreground">{t('settings.tlsCard.compareFingerprintWhatBrowser')}</p>
          </>
        ) : (
          <Alert tone={s.mode === 'off' ? 'warning' : 'info'} title={t('settings.tlsCard.noHttpsCertificate')}>{rich('settings.tlsCard.panelServesPlainHttp', { code: (c) => <code className="font-mono">{c}</code>, code2: (c) => <code className="font-mono">{c}</code> })}</Alert>
        )}
        {s.error && <Alert tone="warning" title={t('settings.tlsCard.httpsProblem')}>{s.error}</Alert>}
        {soon && <Alert tone="warning">{t('settings.tlsCard.certificateEndsSoonValue', { value: s.source === 'provided' ? t('settings.tlsCard.replaceFilesPanelReads') : t('settings.tlsCard.koraRenewsItselfUse') })}</Alert>}
        {s.proxy_trusted && <p className="text-muted-foreground">{rich('settings.tlsCard.reverseProxyTrustedCode', { code: (c) => <code className="font-mono">{c}</code> })}</p>}
        {!role.remote && s.active && s.source !== 'provided' && (
          <div className="flex flex-wrap gap-3">
            <Button variant="secondary" disabled={renew.isPending} onClick={() => renew.mutate()}>{t('settings.tlsCard.renewNow')}</Button>
            {s.ca_fingerprint && <Button variant="secondary" disabled={get.isPending} onClick={() => get.mutate()}>{t('settings.tlsCard.showAuthoritysCertificate')}</Button>}
          </div>
        )}
        {ca && (
          <div className="space-y-2">
            <p>{rich('settings.tlsCard.installCertificateAuthorityOnc', { code: (c) => <code className="font-mono">{c}</code> })}</p>
            <CopyField value={ca} label={t('settings.tlsCard.certificateAuthorityPem')} />
            <ul className="list-disc space-y-1 pl-5 text-xs text-muted-foreground">
              {TRUST_STEPS.map(([a, b]) => <li key={a}><strong>{a}:</strong> {b}</li>)}
            </ul>
          </div>
        )}
        <ErrorNotice error={renew.error ?? get.error} />
      </CardContent>
    </Card>
  )
}
