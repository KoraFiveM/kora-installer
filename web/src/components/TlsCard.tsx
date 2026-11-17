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
            <p>
              The panel uses {SOURCE[s.source ?? ''] ?? 'a certificate'}. <Badge tone={soon ? 'amber' : 'plain'}>{s.days_left !== null && s.days_left >= 0 ? `${s.days_left} days left` : 'expired'}</Badge>
            </p>
            <dl className="grid gap-x-4 gap-y-1 sm:grid-cols-[10rem_1fr]">
              <dt className="text-muted-foreground">Valid until</dt><dd>{when(s.not_after)}</dd>
              <dt className="text-muted-foreground">Names</dt><dd>{s.names.join(', ')}</dd>
              <dt className="text-muted-foreground">Fingerprint (SHA-256)</dt><dd><code className="break-all font-mono text-xs">{s.fingerprint}</code></dd>
              {s.ca_fingerprint && (<><dt className="text-muted-foreground">Local authority</dt><dd><code className="break-all font-mono text-xs">{s.ca_fingerprint}</code></dd></>)}
            </dl>
            <p className="text-xs text-muted-foreground">Compare the fingerprint with what your browser shows for the certificate (click the padlock, then the certificate). Plain HTTP on this computer's own address stays as it is: no warnings there.</p>
          </>
        ) : (
          <Alert tone={s.mode === 'off' ? 'warning' : 'info'} title="No HTTPS certificate">
            The panel serves plain HTTP. That is fine on this computer and inside Tailscale. On a home network or the internet, set up HTTPS: run <code className="font-mono">kora panel tls self-signed</code> for a certificate made on this computer, or <code className="font-mono">kora panel tls set --cert &lt;file&gt; --key &lt;file&gt;</code> for your own.
          </Alert>
        )}
        {s.error && <Alert tone="warning" title="HTTPS problem">{s.error}</Alert>}
        {soon && <Alert tone="warning">The certificate ends soon. {s.source === 'provided' ? 'Replace the files; the panel reads them again by itself.' : 'Kora renews it by itself; use Renew now if it does not.'}</Alert>}
        {s.proxy_trusted && <p className="text-muted-foreground">A reverse proxy is trusted (<code className="font-mono">kora panel proxy status</code>).</p>}
        {!role.remote && s.active && s.source !== 'provided' && (
          <div className="flex flex-wrap gap-3">
            <Button variant="secondary" disabled={renew.isPending} onClick={() => renew.mutate()}>Renew now</Button>
            {s.ca_fingerprint && <Button variant="secondary" disabled={get.isPending} onClick={() => get.mutate()}>Show the authority's certificate</Button>}
          </div>
        )}
        {ca && (
          <div className="space-y-2">
            <p>Install this certificate authority once, so browsers trust the panel. Compare its fingerprint with the one above first. Or use <code className="font-mono">kora panel tls export-ca</code> to save it as a file.</p>
            <CopyField value={ca} label="Certificate authority (PEM)" />
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
