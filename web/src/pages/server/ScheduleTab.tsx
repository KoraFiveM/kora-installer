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
import { t } from '@/i18n'
import '@/i18n/en/servers'

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
        return t('servers.scheduleTab.automaticUpdatesOff')
      }
      const cfg: ScheduleConfig = { preset: kind, time, weekday: kind === 'weekly' ? day : null, run_as: runAs, enabled: true }
      await api.schedule.set(s.name, cfg)
      return t('servers.scheduleTab.savedServerUpdatesItself', { value: kind === 'weekly' ? DAYS[day] : 'day', time })
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ['schedule', s.name] }),
  })
  const bad = kind !== 'off' && !/^([01]\d|2[0-3]):[0-5]\d$/.test(time)
  return (
    <Card className="max-w-2xl">
      <CardHeader><CardTitle>{t('servers.scheduleTab.automaticUpdates')}</CardTitle></CardHeader>
      <CardContent className="space-y-5">
        <p className="text-sm text-muted-foreground">{t('servers.scheduleTab.everyServerUpdateItself')}</p>
        {q.data?.description && <Alert tone="info" title={t('servers.scheduleTab.currentSchedule')}>{q.data.description}</Alert>}
        <fieldset className="space-y-2">
          <legend className="mb-1 text-sm font-semibold">{t('servers.scheduleTab.howOften')}</legend>
          <Radio name="kind" checked={kind === 'off'} onChange={() => setKind('off')} label={t('servers.scheduleTab.off')} hint={t('servers.scheduleTab.updatesOnlyHappenWhen')} />
          <Radio name="kind" checked={kind === 'daily'} onChange={() => setKind('daily')} label={t('servers.scheduleTab.everyDay')} />
          <Radio name="kind" checked={kind === 'weekly'} onChange={() => setKind('weekly')} label={t('servers.scheduleTab.onceWeek')} />
        </fieldset>
        {kind !== 'off' && (
          <>
            <div className="flex flex-wrap gap-4">
              {kind === 'weekly' && <div className="w-44"><Field label={t('servers.scheduleTab.day')}><Select value={day} onChange={(e) => setDay(Number(e.target.value))}>{DAYS.map((d, i) => <option key={d} value={i}>{d}</option>)}</Select></Field></div>}
              <div className="w-32"><Field label={t('servers.scheduleTab.timeHours')} error={bad ? t('servers.scheduleTab.useFormHhMm') : null}><Input value={time} onChange={(e) => setTime(e.target.value)} inputMode="numeric" placeholder="04:00" /></Field></div>
            </div>
            <fieldset className="space-y-2">
              <legend className="mb-1 text-sm font-semibold">{t('servers.scheduleTab.runUpdate')}</legend>
              <Radio name="runas" checked={runAs === 'installing_user'} onChange={() => setRunAs('installing_user')} label={t('servers.scheduleTab.you')} hint={t('servers.scheduleTab.usesGithubSigninPrivate')} />
              <Radio name="runas" checked={runAs === 'system'} onChange={() => setRunAs('system')} label={t('servers.scheduleTab.systemAccount')} hint={t('servers.scheduleTab.runsEvenWhenNobody')} />
            </fieldset>
          </>
        )}
        <Button variant="default" disabled={save.isPending || bad || !!blocked} title={blocked ?? undefined} onClick={() => save.mutate()}>{kind === 'off' ? t('servers.scheduleTab.turnScheduleOff') : t('servers.scheduleTab.saveSchedule')}</Button>
        {blocked && <p className="text-xs text-muted-foreground">{blocked}</p>}
        {save.isSuccess && <Alert tone="success">{save.data}</Alert>}
        <ErrorNotice error={save.error} />
      </CardContent>
    </Card>
  )
}
