import { Fragment, useState, type ReactNode } from 'react'
import { useInfiniteQuery, useQuery } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { AuditRow, ServiceAuditRow } from '@/api/types'
import { opLabel, serviceWhat, whoLabel } from '@/api/store'
import { Badge, type Tone } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input, Select } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { useRole } from '@/hooks/useRole'
import { when } from '@/components/JobHistory'

const RESULT: Record<string, { label: string; tone: Tone }> = {
  ok: { label: 'Done', tone: 'success' },
  started: { label: 'Started', tone: 'info' },
  failed: { label: 'Failed', tone: 'danger' },
  refused: { label: 'Not allowed', tone: 'amber' },
}

function Line({ r }: { r: AuditRow }) {
  const res = RESULT[r.result] ?? { label: r.result, tone: 'neutral' as Tone }
  return (
    <tr className="align-top" data-testid="audit-row">
      <td className="whitespace-nowrap py-2 pr-3 text-xs text-muted-foreground">{when(r.at)}</td>
      <td className="py-2 pr-3 text-sm">{whoLabel(r)}</td>
      <td className="py-2 pr-3 text-sm">{opLabel(r.op)}{r.server ? <span className="text-muted-foreground"> · {r.server}</span> : null}{r.detail ? <span className="block text-xs text-muted-foreground">{r.detail}</span> : null}</td>
      <td className="py-2"><Badge tone={res.tone}>{res.label}</Badge></td>
    </tr>
  )
}

const SERVICE_RESULT: Record<string, { label: string; tone: Tone }> = {
  done: { label: 'Done', tone: 'success' },
  failed: { label: 'Failed', tone: 'danger' },
  refused: { label: 'Not allowed', tone: 'amber' },
  stopped: { label: 'Stopped', tone: 'danger' },
}
const SERVICE_TO_PANEL: Record<string, string> = { done: 'ok', failed: 'failed', refused: 'refused' }

/** One thing the administrator service did, in the same table as the panel's own rows. */
function ServiceLine({ r }: { r: ServiceAuditRow }) {
  const res = SERVICE_RESULT[r.result] ?? { label: r.result, tone: 'neutral' as Tone }
  const w = serviceWhat(r.what)
  return (
    <tr className="align-top" data-testid="audit-row" data-source="service">
      <td className="whitespace-nowrap py-2 pr-3 text-xs text-muted-foreground">{when(r.at)}</td>
      <td className="py-2 pr-3 text-sm">{r.account}</td>
      <td className="py-2 pr-3 text-sm">{w.text}{w.server ? <span className="text-muted-foreground"> · {w.server}</span> : null}<span className="block text-xs text-muted-foreground">Done with administrator rights{r.detail ? ` · ${r.detail}` : ''}</span></td>
      <td className="py-2"><Badge tone={res.tone}>{res.label}</Badge></td>
    </tr>
  )
}

/** Who changed what (administrators only). Nothing secret is recorded. */
export function AuditLogCard() {
  const role = useRole()
  const [server, setServer] = useState('')
  const [actor, setActor] = useState('')
  const [result, setResult] = useState('')
  const q = useInfiniteQuery({
    queryKey: ['audit', server, actor, result],
    queryFn: ({ pageParam }) => api.audit.list({ limit: 50, before: pageParam, server, actor, result }),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (last) => (last.more && last.entries.length ? last.entries[last.entries.length - 1].id : undefined),
    enabled: role.can('admin'),
  })
  // The administrator service keeps its own log; it is shown in the same table, merged by time. A failure to read it never hides the panel's own rows.
  const svc = useQuery({ queryKey: ['audit-service'], queryFn: () => api.auditService.list(200), enabled: role.can('admin'), retry: false })
  if (!role.can('admin')) return null
  const rows = q.data?.pages.flatMap((p) => p.entries) ?? []
  const days = q.data?.pages[0]?.retention_days
  // Older panel rows may still be on the next page: do not show service rows older than the oldest panel row shown, or the order would lie.
  const oldestShown = q.hasNextPage && rows.length ? rows[rows.length - 1].at : 0
  const serviceRows = (svc.data?.entries ?? []).filter(
    (r) =>
      r.at >= oldestShown &&
      (!server || r.what.toLowerCase().includes(server.toLowerCase())) &&
      (!actor || r.account.toLowerCase().includes(actor.toLowerCase())) &&
      (!result || SERVICE_TO_PANEL[r.result] === result),
  )
  type Merged = { at: number; key: string; node: ReactNode }
  const merged: Merged[] = [
    ...rows.map((r) => ({ at: r.at, key: `p${r.id}`, node: <Line r={r} /> })),
    ...serviceRows.map((r, i) => ({ at: r.at, key: `s${r.at}-${i}`, node: <ServiceLine r={r} /> })),
  ].sort((x, y) => y.at - x.at)
  return (
    <Card>
      <CardHeader><CardTitle>Activity log</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">Who changed what, from this panel (on this computer or from another device) and from <code>kora</code> commands. Passwords, licence keys and what was typed are never recorded. What Kora&apos;s administrator service did with administrator rights (services, firewall rules, databases, updates, scheduled backups) is kept in its own log, <code>%ProgramFiles%\Kora\data\audit.log</code> on Windows and <code>/var/log/kora/audit.log</code> on Linux. Those entries are shown here too, marked &ldquo;Done with administrator rights&rdquo;.{days ? ` The panel&apos;s entries are kept for ${days} days.` : ''}</p>
        {svc.data && !svc.data.available && svc.data.note && <p className="text-sm text-muted-foreground" data-testid="service-audit-note">{svc.data.note}</p>}
        {svc.data?.available && svc.data.note && <p className="text-sm text-muted-foreground" data-testid="service-audit-note">{svc.data.note}</p>}
        {svc.data?.available && !svc.data.all_accounts && <p className="text-xs text-muted-foreground">Of the administrator service&apos;s log, only what it did for your account is shown. An administrator of this computer sees everyone&apos;s.</p>}
        <div className="grid gap-3 sm:grid-cols-3">
          <Field label="Server"><Input value={server} onChange={(e) => setServer(e.target.value)} placeholder="Any server" /></Field>
          <Field label="Who"><Input value={actor} onChange={(e) => setActor(e.target.value)} placeholder="Any name" /></Field>
          <Field label="Result">
            <Select value={result} onChange={(e) => setResult(e.target.value)}>
              <option value="">Any</option>
              <option value="ok">Done</option>
              <option value="started">Started</option>
              <option value="failed">Failed</option>
              <option value="refused">Not allowed</option>
            </Select>
          </Field>
        </div>
        {q.isPending && <p className="text-sm text-muted-foreground">Loading...</p>}
        <ErrorNotice error={q.error} />
        {q.data && merged.length === 0 && <p className="text-sm">Nothing matches. {server || actor || result ? 'Clear the filters to see everything.' : 'Changes will appear here.'}</p>}
        {merged.length > 0 && (
          <div className="overflow-x-auto">
            <table className="w-full text-left">
              <thead><tr className="text-xs text-muted-foreground"><th className="pb-1 pr-3 font-medium">When</th><th className="pb-1 pr-3 font-medium">Who</th><th className="pb-1 pr-3 font-medium">What</th><th className="pb-1 font-medium">Result</th></tr></thead>
              <tbody className="divide-y divide-border">{merged.map((m) => <Fragment key={m.key}>{m.node}</Fragment>)}</tbody>
            </table>
          </div>
        )}
        {q.hasNextPage && <Button size="sm" onClick={() => void q.fetchNextPage()} disabled={q.isFetchingNextPage}>Show older entries</Button>}
      </CardContent>
    </Card>
  )
}
