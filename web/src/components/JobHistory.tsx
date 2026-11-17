import { Link } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { storeApi, opLabel, type HistoryState, type JobSummary } from '@/api/store'
import { Badge, type Tone } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ErrorNotice } from '@/components/ErrorNotice'

const STATE: Record<HistoryState, { label: string; tone: Tone }> = {
  running: { label: 'Running', tone: 'info' },
  done: { label: 'Finished', tone: 'success' },
  failed: { label: 'Did not finish', tone: 'danger' },
  interrupted: { label: 'Stopped by a restart', tone: 'amber' },
}

export const when = (secs: number) => new Date(secs * 1000).toLocaleString()

function Row({ j }: { j: JobSummary }) {
  const s = STATE[j.state] ?? STATE.failed
  return (
    <li className="space-y-2 py-3" data-testid="job-row">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span className="font-medium">{opLabel(j.op)}</span>
        {j.server && <span className="text-muted-foreground">{j.server}</span>}
        <Badge tone={s.tone}>{s.label}</Badge>
        <span className="ml-auto text-xs text-muted-foreground">{when(j.started)}</span>
      </div>
      {(j.state === 'failed' || j.state === 'interrupted') && j.error && <ErrorNotice error={j.error} />}
      {j.state === 'interrupted' && j.error?.resume && (
        <p className="text-sm">
          <Link className="text-info underline" to="/install">Open Install to carry on</Link>
        </p>
      )}
    </li>
  )
}

/** What happened recently, also from before Kora Panel last restarted. */
export function JobHistoryCard() {
  const q = useQuery({ queryKey: ['job-history'], queryFn: () => storeApi.jobs(30), refetchInterval: 15_000 })
  return (
    <Card>
      <CardHeader><CardTitle>Recent jobs</CardTitle></CardHeader>
      <CardContent className="space-y-2">
        <p className="text-sm text-muted-foreground">Updates, installs and other jobs you started. They are kept when Kora Panel restarts. A job that was still running when the panel stopped shows as stopped by a restart, with what to do next.</p>
        {q.isPending && <p className="text-sm text-muted-foreground">Loading...</p>}
        <ErrorNotice error={q.error} />
        {q.data && q.data.jobs.length === 0 && <p className="text-sm">Nothing has run yet.</p>}
        {q.data && q.data.jobs.length > 0 && <ul className="divide-y divide-border">{q.data.jobs.map((j) => <Row key={j.id} j={j} />)}</ul>}
      </CardContent>
    </Card>
  )
}
