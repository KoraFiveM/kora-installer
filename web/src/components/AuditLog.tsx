import { useState } from 'react'
import { useInfiniteQuery } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { AuditRow } from '@/api/types'
import { opLabel, whoLabel } from '@/api/store'
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
  if (!role.can('admin')) return null
  const rows = q.data?.pages.flatMap((p) => p.entries) ?? []
  const days = q.data?.pages[0]?.retention_days
  return (
    <Card>
      <CardHeader><CardTitle>Activity log</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">Who changed what, from this panel (on this computer or from another device) and from <code>kora</code> commands. Passwords, licence keys and what was typed are never recorded. What Kora&apos;s administrator service did with administrator rights (services, firewall rules, databases, updates) is kept in its own log that only administrators can read: <code>%ProgramFiles%\Kora\data\audit.log</code> on Windows, <code>/var/log/kora/audit.log</code> on Linux.{days ? ` Entries are kept for ${days} days.` : ''}</p>
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
        {q.isPending && <p role="status" className="text-sm text-muted-foreground">Loading...</p>}
        <ErrorNotice error={q.error} />
        {q.data && rows.length === 0 && <p className="text-sm">Nothing matches. {server || actor || result ? 'Clear the filters to see everything.' : 'Changes will appear here.'}</p>}
        {rows.length > 0 && (
          <div className="overflow-x-auto" tabIndex={0} role="region" aria-label="Activity log entries">
            <table className="w-full text-left">
              <caption className="sr-only">Activity log, newest first</caption>
              <thead><tr className="text-xs text-muted-foreground"><th scope="col" className="pb-1 pr-3 font-medium">When</th><th scope="col" className="pb-1 pr-3 font-medium">Who</th><th scope="col" className="pb-1 pr-3 font-medium">What</th><th scope="col" className="pb-1 font-medium">Result</th></tr></thead>
              <tbody className="divide-y divide-border">{rows.map((r) => <Line key={r.id} r={r} />)}</tbody>
            </table>
          </div>
        )}
        {q.hasNextPage && <Button size="sm" onClick={() => void q.fetchNextPage()} disabled={q.isFetchingNextPage}>Show older entries</Button>}
      </CardContent>
    </Card>
  )
}
