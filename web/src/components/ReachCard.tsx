import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { CheckCircle2, CircleHelp, Loader2, ShieldAlert, ThumbsUp } from 'lucide-react'
import { reachApi, type ReachFix, type ReachLayer, type ReachReport, type ReachState } from '@/api/reach'
import { Badge, type Tone } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Alert } from '@/components/ui/alert'
import { Checkbox } from '@/components/ui/checkbox'
import { CopyField } from '@/components/CopyButton'
import { ErrorNotice } from '@/components/ErrorNotice'
import { useRole, whyNot } from '@/hooks/useRole'

const TONE: Record<ReachState, Tone> = { ok: 'success', likely: 'success', problem: 'danger', unknown: 'neutral' }
const LABEL: Record<ReachState, string> = { ok: 'OK', likely: 'Probably OK', problem: 'Problem', unknown: 'Not known' }

/** The last check of each server, so other places (the connect address, the wizard) can reuse it. */
export const reachKey = (name: string) => ['reach', name] as const

function Icon({ state }: { state: ReachState }) {
  if (state === 'ok') return <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-success" aria-hidden />
  if (state === 'likely') return <ThumbsUp className="mt-0.5 size-4 shrink-0 text-success" aria-hidden />
  if (state === 'problem') return <ShieldAlert className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
  return <CircleHelp className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
}

/** What turning on port forwarding means, in one honest sentence plus the limits Kora keeps. */
export const FORWARDING_RISK =
  "A router that programs can reconfigure is a small risk: any program on your network can do the same. Turning this on lets Kora do it for this server's game port only (TCP and UDP): it keeps the rule while the server runs and removes it when the server stops."

/** What each outside lookup contacts and learns, shown before it is allowed. */
const CONSENT_TEXT = {
  cfx_list: { what: "Cfx.re's public server list (frontend.cfx-services.net)", learns: "Kora sends your server's join code (the one from its own log). Cfx.re already knows it; Kora learns whether you are listed and which address Cfx.re has.", button: 'Allow and check again' },
  ip_echo: { what: 'api.ipify.org', learns: "That website sees this computer's public address and that a request was made. Kora learns your public address.", button: 'Allow and check again' },
} as const

function Layer({ l, fixes }: { l: ReachLayer; fixes: React.ReactNode }) {
  return (
    <li className="flex gap-3 rounded-md border border-border p-3" data-state={l.state} data-layer={l.id}>
      <Icon state={l.state} />
      <div className="min-w-0 flex-1 space-y-1.5">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-medium">{l.title}</span>
          <Badge tone={TONE[l.state]} dot>{LABEL[l.state]}</Badge>
        </div>
        {l.detail && <p className="text-sm text-muted-foreground">{l.detail}</p>}
        {l.steps.length > 0 && (
          <ol className="list-decimal space-y-1 pl-5 text-sm">{l.steps.map((s, i) => <li key={i}>{s}</li>)}</ol>
        )}
        {fixes}
      </div>
    </li>
  )
}

/**
 * "Can players reach this server?": a checklist with one line per layer (running, firewall, router, internet), each
 * OK / problem / unknown, with a fix button where Kora can act and exact instructions where it cannot.
 * Nothing outside this computer is contacted unless the person allowed it, and the card says what first.
 */
export function ReachCard({ name, onStart, compact = false }: { name: string; onStart?: () => void; compact?: boolean }) {
  const qc = useQueryClient()
  const role = useRole()
  const [report, setReport] = useState<ReachReport | null>(() => qc.getQueryData<ReachReport>(reachKey(name)) ?? null)
  const [asking, setAsking] = useState<'cfx_list' | 'ip_echo' | 'forwarding' | null>(null)
  const [done, setDone] = useState<string | null>(null)

  const check = useMutation({
    mutationFn: (once?: { cfx_list?: boolean; ip_echo?: boolean }) => reachApi.check(name, once),
    onSuccess: (r) => {
      setReport(r)
      qc.setQueryData(reachKey(name), r)
    },
  })
  const act = useMutation({
    mutationFn: async (fix: ReachFix | 'forwarding_off') => {
      setDone(null)
      if (fix === 'forwarding_off') await reachApi.setForwarding(name, false)
      else if (fix.id === 'enable_forwarding') await reachApi.setForwarding(name, true)
      else if (fix.id === 'open_firewall') await reachApi.openFirewall(name)
      else if (fix.id === 'allow_cfx_list') await reachApi.setSettings({ cfx_list: true })
      else if (fix.id === 'allow_ip_echo') await reachApi.setSettings({ ip_echo: true })
      else if (fix.id === 'start_server') onStart?.()
    },
    onSuccess: () => {
      setAsking(null)
      check.mutate(undefined)
    },
  })

  const checkOff = whyNot(role, 'operator')
  const adminOff = whyNot(role, 'admin')
  const busy = check.isPending || act.isPending

  const fixButtons = (l: ReachLayer) =>
    l.fixes
      .filter((f) => f.id !== 'start_server' || onStart)
      .map((f) => {
        if (f.id === 'allow_cfx_list' || f.id === 'allow_ip_echo') {
          const key = f.id === 'allow_cfx_list' ? 'cfx_list' : 'ip_echo'
          return (
            <div key={f.id} className="space-y-2">
              {asking === key ? (
                <Alert tone="info" title={`This will contact ${CONSENT_TEXT[key].what}`}>
                  <p>{CONSENT_TEXT[key].learns}</p>
                  <p className="mt-1">Your choice is remembered; you can turn it off again at the bottom of this card.</p>
                  <div className="mt-2 flex gap-2">
                    <Button size="sm" disabled={busy || adminOff !== null} title={adminOff ?? undefined} onClick={() => act.mutate(f)}>{CONSENT_TEXT[key].button}</Button>
                    <Button size="sm" variant="ghost" onClick={() => setAsking(null)}>Not now</Button>
                    <Button size="sm" variant="ghost" disabled={busy} onClick={() => { setAsking(null); check.mutate({ [key]: true }) }}>Just this once</Button>
                  </div>
                </Alert>
              ) : (
                <Button size="sm" disabled={busy} onClick={() => setAsking(key)}>{f.label}</Button>
              )}
            </div>
          )
        }
        if (f.id === 'enable_forwarding') {
          return (
            <div key={f.id} className="space-y-2">
              {asking === 'forwarding' ? (
                <Alert tone="warning" title="Let Kora change your router?">
                  <p>{FORWARDING_RISK}</p>
                  <div className="mt-2 flex gap-2">
                    <Button size="sm" disabled={busy || adminOff !== null} title={adminOff ?? undefined} onClick={() => act.mutate(f)}>Turn on port forwarding</Button>
                    <Button size="sm" variant="ghost" onClick={() => setAsking(null)}>Not now</Button>
                  </div>
                </Alert>
              ) : (
                <Button size="sm" disabled={busy || adminOff !== null} title={adminOff ?? undefined} onClick={() => setAsking('forwarding')}>{f.label}</Button>
              )}
            </div>
          )
        }
        return (
          <Button key={f.id} size="sm" disabled={busy || (f.id !== 'start_server' && adminOff !== null)} title={f.id !== 'start_server' ? adminOff ?? undefined : undefined} onClick={() => act.mutate(f)}>
            {f.label}
          </Button>
        )
      })

  return (
    <Card>
      <CardHeader><CardTitle>Can players reach this server?</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        {!report && !check.isPending && (
          <p className="text-sm text-muted-foreground">
            Kora checks four things in turn: is the server listening, does this computer's firewall let players in, does your router send them here, and can the internet reach it. It only looks at this computer and your router. Nothing outside is contacted unless you allow it.
          </p>
        )}
        <div className="flex flex-wrap items-center gap-2">
          <Button size="default" disabled={busy || checkOff !== null} title={checkOff ?? undefined} onClick={() => check.mutate(undefined)}>
            {check.isPending ? <Loader2 className="animate-spin" /> : null} {report ? 'Check again' : 'Check now'}
          </Button>
          {check.isPending && <span className="text-sm text-muted-foreground">Looking at the server, the firewall and your router (a few seconds)…</span>}
        </div>
        {check.error && <ErrorNotice error={check.error} onRetry={() => check.mutate(undefined)} />}
        {act.error && <ErrorNotice error={act.error} onRetry={() => act.reset()} />}
        {done && <p className="text-sm text-success">{done}</p>}
        {report && (
          <>
            <Alert tone={report.verdict === 'problem' ? 'warning' : report.verdict === 'ok' ? 'success' : 'info'}>{report.headline}</Alert>
            <ul className="space-y-2">
              {report.layers.map((l) => <Layer key={l.id} l={l} fixes={<div className="flex flex-col items-start gap-2 pt-1">{fixButtons(l)}</div>} />)}
            </ul>
            {report.forwarding.enabled && (
              <div className="flex flex-wrap items-center gap-2 text-sm">
                <span className="text-muted-foreground">Automatic port forwarding is on.</span>
                <Button size="sm" variant="ghost" disabled={busy || adminOff !== null} title={adminOff ?? undefined} onClick={() => act.mutate('forwarding_off')}>Turn it off</Button>
              </div>
            )}
            {!compact && (report.connect.lan || report.connect.public || report.connect.cfx) && (
              <div className="space-y-2">
                <div className="text-sm text-muted-foreground">Addresses to give people</div>
                {report.connect.lan && <CopyField value={report.connect.lan} label="Address on your own network" />}
                {report.connect.public && <CopyField value={report.connect.public} label="Address on the internet" />}
                {report.connect.cfx && <CopyField value={report.connect.cfx} label="Cfx.re join link" />}
              </div>
            )}
            <div className="space-y-1.5 border-t border-border pt-3">
              <div className="text-sm text-muted-foreground">Outside lookups Kora may do for this check (off unless you allow them)</div>
              <Checkbox label="Cfx.re's public server list" hint="Asks frontend.cfx-services.net about this server's own join code." checked={report.consent.cfx_list} disabled={busy || adminOff !== null} onChange={(e) => { setDone(null); void reachApi.setSettings({ cfx_list: e.target.checked }).then(() => check.mutate(undefined)) }} />
              <Checkbox label="A public-address website (api.ipify.org)" hint="Only used when the router cannot tell Kora the public address. It sees this computer's public address." checked={report.consent.ip_echo} disabled={busy || adminOff !== null} onChange={(e) => { setDone(null); void reachApi.setSettings({ ip_echo: e.target.checked }).then(() => check.mutate(undefined)) }} />
            </div>
            {report.contacted.length > 0 && (
              <ul className="space-y-1 text-xs text-muted-foreground">
                {report.contacted.map((c, i) => <li key={i}>This check asked {c.what}; it learned {c.learned}.</li>)}
              </ul>
            )}
          </>
        )}
      </CardContent>
    </Card>
  )
}
