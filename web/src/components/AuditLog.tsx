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
import { t, msg, type Msg } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/core/auditLog'
import '@/i18n/en/settings/auditLog'

const RESULT: Record<string, { label: Msg | null; tone: Tone }> = {
  ok: { label: msg('core.auditLog.done'), tone: 'success' },
  started: { label: msg('core.auditLog.started'), tone: 'info' },
  failed: { label: msg('core.auditLog.failed'), tone: 'danger' },
  refused: { label: msg('core.auditLog.notAllowed'), tone: 'amber' },
}

function Line({ r }: { r: AuditRow }) {
  const res = RESULT[r.result] ?? { label: null, tone: 'neutral' as Tone }
  return (
    <tr className="align-top" data-testid="audit-row">
      <td className="whitespace-nowrap py-2 pr-3 text-xs text-muted-foreground">{when(r.at)}</td>
      <td className="py-2 pr-3 text-sm">{whoLabel(r)}</td>
      <td className="py-2 pr-3 text-sm">{opLabel(r.op)}{r.server ? <span className="text-muted-foreground"> · {r.server}</span> : null}{r.detail ? <span className="block text-xs text-muted-foreground">{r.detail}</span> : null}</td>
      <td className="py-2"><Badge tone={res.tone}>{res.label ? t(res.label) : r.result}</Badge></td>
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
      <CardHeader><CardTitle>{t('settings.auditLog.activityLog')}</CardTitle></CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">{rich('settings.auditLog.whoChangedWhatFrom', { value: days ? t('settings.auditLog.entriesKeptDaysDays', { days }) : '', code: (c) => <code>{c}</code>, code2: (c) => <code>{c}</code>, code3: (c) => <code>{c}</code> })}</p>
        <div className="grid gap-3 sm:grid-cols-3">
          <Field label={t('settings.auditLog.server')}><Input value={server} onChange={(e) => setServer(e.target.value)} placeholder={t('settings.auditLog.anyServer')} /></Field>
          <Field label={t('settings.auditLog.who')}><Input value={actor} onChange={(e) => setActor(e.target.value)} placeholder={t('settings.auditLog.anyName')} /></Field>
          <Field label={t('settings.auditLog.result')}>
            <Select value={result} onChange={(e) => setResult(e.target.value)}>
              <option value="">{t('settings.auditLog.any')}</option>
              <option value="ok">{t('settings.auditLog.done')}</option>
              <option value="started">{t('settings.auditLog.started')}</option>
              <option value="failed">{t('settings.auditLog.failed')}</option>
              <option value="refused">{t('settings.auditLog.notAllowed')}</option>
            </Select>
          </Field>
        </div>
        {q.isPending && <p role="status" className="text-sm text-muted-foreground">{t('settings.auditLog.loading')}</p>}
        <ErrorNotice error={q.error} />
        {q.data && rows.length === 0 && <p className="text-sm">{t('settings.auditLog.nothingMatchesValue', { value: server || actor || result ? t('settings.auditLog.clearFiltersSeeEverything') : t('settings.auditLog.changesAppearHere') })}</p>}
        {rows.length > 0 && (
          <div className="overflow-x-auto" tabIndex={0} role="region" aria-label={t('settings.auditLog.activityLogEntries')}>
            <table className="w-full text-left">
              <caption className="sr-only">{t('settings.auditLog.activityLogNewestFirst')}</caption>
              <thead><tr className="text-xs text-muted-foreground"><th scope="col" className="pb-1 pr-3 font-medium">{t('settings.auditLog.when')}</th><th scope="col" className="pb-1 pr-3 font-medium">{t('settings.auditLog.who')}</th><th scope="col" className="pb-1 pr-3 font-medium">{t('settings.auditLog.what')}</th><th scope="col" className="pb-1 font-medium">{t('settings.auditLog.result')}</th></tr></thead>
              <tbody className="divide-y divide-border">{rows.map((r) => <Line key={r.id} r={r} />)}</tbody>
            </table>
          </div>
        )}
        {q.hasNextPage && <Button size="sm" onClick={() => void q.fetchNextPage()} disabled={q.isFetchingNextPage}>{t('settings.auditLog.showOlderEntries')}</Button>}
      </CardContent>
    </Card>
  )
}
