import { Link } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { JobState, JobSummary } from '@/api/types'
import { opLabel } from '@/api/store'
import { Badge, type Tone } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ErrorNotice } from '@/components/ErrorNotice'
import { t, msg, type Msg, fmt, unix } from '@/i18n'
import '@/i18n/en/core'

const STATE: Record<JobState, { label: Msg; tone: Tone }> = {
  running: { label: msg('core.jobHistory.running'), tone: 'info' },
  done: { label: msg('core.jobHistory.finished'), tone: 'success' },
  failed: { label: msg('core.jobHistory.didFinish'), tone: 'danger' },
  interrupted: { label: msg('core.jobHistory.stoppedRestart'), tone: 'amber' },
}

export const when = (secs: number) => fmt.dateTime(unix(secs))

function Row({ j }: { j: JobSummary }) {
  const s = STATE[j.state] ?? STATE.failed
  return (
    <li className="space-y-2 py-3" data-testid="job-row">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span className="font-medium">{opLabel(j.op)}</span>
        {j.server && <span className="text-muted-foreground">{j.server}</span>}
        <Badge tone={s.tone}>{t(s.label)}</Badge>
        <span className="ml-auto text-xs text-muted-foreground">{when(j.started)}</span>
      </div>
      {(j.state === 'failed' || j.state === 'interrupted') && j.error && <ErrorNotice error={j.error} />}
      {j.state === 'interrupted' && j.error?.resume && (
        <p className="text-sm">
          <Link className="text-info underline" to="/install">{t('core.jobHistory.openInstallCarry')}</Link>
        </p>
      )}
    </li>
  )
}

/** What happened recently, also from before Kora Panel last restarted. */
export function JobHistoryCard() {
  const q = useQuery({ queryKey: ['job-history'], queryFn: () => api.jobs.list(30), refetchInterval: 15_000 })
  return (
    <Card>
      <CardHeader><CardTitle>{t('core.jobHistory.recentJobs')}</CardTitle></CardHeader>
      <CardContent className="space-y-2">
        <p className="text-sm text-muted-foreground">{t('core.jobHistory.updatesInstallsOtherJobs')}</p>
        {q.isPending && <p className="text-sm text-muted-foreground">{t('core.jobHistory.loading')}</p>}
        <ErrorNotice error={q.error} />
        {q.data && q.data.length === 0 && <p className="text-sm">{t('core.jobHistory.nothingRunYet')}</p>}
        {q.data && q.data.length > 0 && <ul className="divide-y divide-border">{q.data.map((j) => <Row key={j.id} j={j} />)}</ul>}
      </CardContent>
    </Card>
  )
}
