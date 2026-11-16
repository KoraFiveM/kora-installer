import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { RunAs, ScheduleConfig, ServerDetail } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input, Select } from '@/components/ui/input'
import { Radio } from '@/components/ui/checkbox'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'

const DAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday']
type Kind = 'off' | 'daily' | 'weekly'

export function ScheduleTab({ server: s }: { server: ServerDetail }) {
  const qc = useQueryClient()
  const role = useRole()
  const blocked = whyNot(role, 'admin', true)
  const q = useQuery({ queryKey: ['schedule', s.name], queryFn: () => api.schedule.get(s.name) })
  const [kind, setKind] = useState<Kind>('off')
  const [time, setTime] = useState('04:00')
  const [day, setDay] = useState(0)
  const [runAs, setRunAs] = useState<RunAs>('installing_user')
  useEffect(() => {
    const c = q.data?.schedule
    if (c) {
      setKind(c.preset === 'daily' ? 'daily' : 'weekly')
      setTime(c.time)
      setDay(c.weekday ?? 0)
      setRunAs(c.run_as)
    } else if (q.data) setKind('off')
  }, [q.data])
  const save = useMutation({
    mutationFn: async () => {
      if (kind === 'off') {
        await api.schedule.remove(s.name)
        return 'Automatic updates are off.'
      }
      const cfg: ScheduleConfig = { preset: kind, time, weekday: kind === 'weekly' ? day : null, run_as: runAs, enabled: true }
      await api.schedule.set(s.name, cfg)
      return `Saved. The server updates itself every ${kind === 'weekly' ? DAYS[day] : 'day'} at ${time}.`
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ['schedule', s.name] }),
  })
  const bad = kind !== 'off' && !/^([01]\d|2[0-3]):[0-5]\d$/.test(time)
  return (
    <Card className="max-w-2xl">
      <CardHeader><CardTitle>Automatic updates</CardTitle></CardHeader>
      <CardContent className="space-y-5">
        <p className="text-sm text-muted-foreground">A development server can update itself on a schedule: Kora fetches your forks, updates what is safe to update and checks the server still starts.</p>
        {q.data?.description && <Alert tone="info" title="Current schedule">{q.data.description}</Alert>}
        <fieldset className="space-y-2">
          <legend className="mb-1 text-sm font-semibold">How often</legend>
          <Radio name="kind" checked={kind === 'off'} onChange={() => setKind('off')} label="Off" hint="Updates only happen when you run them yourself." />
          <Radio name="kind" checked={kind === 'daily'} onChange={() => setKind('daily')} label="Every day" />
          <Radio name="kind" checked={kind === 'weekly'} onChange={() => setKind('weekly')} label="Once a week" />
        </fieldset>
        {kind !== 'off' && (
          <>
            <div className="flex flex-wrap gap-4">
              {kind === 'weekly' && <div className="w-44"><Field label="Day"><Select value={day} onChange={(e) => setDay(Number(e.target.value))}>{DAYS.map((d, i) => <option key={d} value={i}>{d}</option>)}</Select></Field></div>}
              <div className="w-32"><Field label="Time (24 hours)" error={bad ? 'Use the form HH:MM, for example 04:00.' : null}><Input value={time} onChange={(e) => setTime(e.target.value)} inputMode="numeric" placeholder="04:00" /></Field></div>
            </div>
            <fieldset className="space-y-2">
              <legend className="mb-1 text-sm font-semibold">Run the update as</legend>
              <Radio name="runas" checked={runAs === 'installing_user'} onChange={() => setRunAs('installing_user')} label="You" hint="Uses your GitHub sign-in, so private forks can be fetched. Runs while your account can run background tasks." />
              <Radio name="runas" checked={runAs === 'system'} onChange={() => setRunAs('system')} label="The system account" hint="Runs even when nobody is signed in, but has no GitHub sign-in, so private forks cannot be fetched. Needs administrator rights to set up." />
            </fieldset>
          </>
        )}
        <Button variant="default" disabled={save.isPending || bad || !!blocked} title={blocked ?? undefined} onClick={() => save.mutate()}>{kind === 'off' ? 'Turn schedule off' : 'Save schedule'}</Button>
        {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
        {save.isSuccess && <Alert tone="success">{save.data}</Alert>}
        <ErrorNotice error={save.error} />
      </CardContent>
    </Card>
  )
}
