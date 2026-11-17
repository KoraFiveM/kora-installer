import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/api/client'
import type { AlertSettings, AlertSettingsUpdate, MetricSample, MetricsRange, ServerDetail } from '@/api/types'
import { useRole, whyNot } from '@/hooks/useRole'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { ErrorNotice } from '@/components/ErrorNotice'
import { Field } from '@/components/Field'
import { LineChart } from '@/components/LineChart'
import { Skeleton } from '@/components/Skeleton'
import { cn } from '@/lib/utils'
import { t as tt, msg, type Msg, fmt } from '@/i18n'
import { rich } from '@/i18n/react'
import '@/i18n/en/servers/monitoringTab'

export const METRICS_POLL_MS = 15000
const RANGES: { id: MetricsRange; label: Msg }[] = [
  { id: '1h', label: msg('servers.monitoringTab.lastHour') },
  { id: '6h', label: msg('servers.monitoringTab.lastHours') },
  { id: '24h', label: msg('servers.monitoringTab.lastHours2') },
]

const gb = (mb: number) => fmt.bytes(mb * 1_000_000)
const mbText = (mb: number) => fmt.bytes(mb * 1_000_000)

function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="rounded-md border border-border p-3">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="mt-1 font-heading text-xl font-semibold">{value}</div>
      {hint && <div className="text-xs text-muted-foreground">{hint}</div>}
    </div>
  )
}

function Current({ s }: { s: MetricSample | null }) {
  const dash = '–'
  return (
    <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
      <Stat label={tt('servers.monitoringTab.players')} value={s?.players === undefined ? dash : String(s.players)} hint={s?.max_players ? tt('servers.monitoringTab.maxPlayers', { max_players: s.max_players }) : undefined} />
      <Stat label={tt('servers.monitoringTab.serverCpu')} value={s?.cpu === undefined ? dash : fmt.percent(s.cpu)} hint={tt('servers.monitoringTab.wholeMachine')} />
      <Stat label={tt('servers.monitoringTab.serverMemory')} value={s?.ram_mb === undefined ? dash : mbText(s.ram_mb)} hint={s?.sys_ram_pct === undefined ? undefined : tt('servers.monitoringTab.roundMachineUse', { round: Math.round(s.sys_ram_pct) })} />
      <Stat label={tt('servers.monitoringTab.freeDiskSpace')} value={s?.disk_free_mb === undefined ? dash : gb(s.disk_free_mb)} hint={s?.disk_free_pct === undefined ? undefined : tt('servers.monitoringTab.roundFree', { round: Math.round(s.disk_free_pct) })} />
    </div>
  )
}

function Charts({ name, range }: { name: string; range: MetricsRange }) {
  const q = useQuery({ queryKey: ['metrics', name, range], queryFn: () => api.metrics.get(name, range), refetchInterval: METRICS_POLL_MS })
  if (q.isLoading) return <Skeleton className="h-64" />
  if (q.isError) return <ErrorNotice error={q.error} />
  const samples = q.data?.samples ?? []
  const series = (f: (s: MetricSample) => number | undefined) => samples.map((s) => ({ t: s.t, v: s.up ? (f(s) ?? null) : null }))
  const latest = q.data?.latest ?? null
  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <Badge tone={latest?.up ? 'success' : 'danger'} dot>{latest ? (latest.up ? tt('servers.monitoringTab.running') : tt('servers.monitoringTab.stopped')) : tt('servers.monitoringTab.noDataYet')}</Badge>
        {latest && <Badge tone={latest.game_up ? 'success' : 'neutral'} dot>{latest.game_up ? tt('servers.monitoringTab.gamePortListening') : tt('servers.monitoringTab.gamePortClosed')}</Badge>}
        {latest && <Badge tone={latest.txadmin_up ? 'success' : 'neutral'} dot>{latest.txadmin_up ? tt('servers.monitoringTab.txadminAnswering') : tt('servers.monitoringTab.txadminAnswering2')}</Badge>}
      </div>
      <Current s={latest} />
      {samples.length === 0 && <p className="text-sm text-muted-foreground">{tt('servers.monitoringTab.nothingBeenMeasuredYet')}</p>}
      <div className="grid gap-5 lg:grid-cols-2">
        <LineChart title={tt('servers.monitoringTab.players')} points={series((s) => s.players)} format={(v) => String(Math.round(v))} tone="text-info" />
        <LineChart title={tt('servers.monitoringTab.serverCpuPercentMachine')} points={series((s) => s.cpu)} max={100} format={(v) => fmt.percent(v, 0)} tone="text-amber" />
        <LineChart title={tt('servers.monitoringTab.serverMemory')} points={series((s) => s.ram_mb)} format={(v) => mbText(v)} tone="text-success" className="lg:col-span-2" />
      </div>
    </div>
  )
}

const num = (s: string) => (s.trim() === '' ? NaN : Number(s))

function SettingsForm({ name, cur }: { name: string; cur: AlertSettings }) {
  const qc = useQueryClient()
  const [enabled, setEnabled] = useState(cur.enabled)
  const [rules, setRules] = useState(cur.rules)
  const [disk, setDisk] = useState(String(cur.disk_min_gb))
  const [diskPct, setDiskPct] = useState(String(cur.disk_min_percent))
  const [ram, setRam] = useState(String(cur.ram_max_percent))
  const [discord, setDiscord] = useState(cur.discord)
  const [url, setUrl] = useState('')
  const [removeUrl, setRemoveUrl] = useState(false)
  const problem =
    !(num(disk) >= 0 && num(disk) <= 100000) ? tt('servers.monitoringTab.freeDiskSpaceMust')
    : !(num(diskPct) >= 0 && num(diskPct) <= 100) ? tt('servers.monitoringTab.diskPercentageMustBetween')
    : !(num(ram) >= 1 && num(ram) <= 100) ? tt('servers.monitoringTab.memoryLimitMustBetween')
    : null
  const save = useMutation({
    mutationFn: () => {
      const body: AlertSettingsUpdate = { enabled, rules, disk_min_gb: num(disk), disk_min_percent: num(diskPct), ram_max_percent: num(ram), discord }
      if (url.trim()) body.webhook_url = url.trim()
      else if (removeUrl) body.webhook_url = ''
      return api.alerts.setSettings(name, body)
    },
    onSuccess: (next) => {
      qc.setQueryData(['alert-settings', name], next)
      setUrl('')
      setRemoveUrl(false)
    },
  })
  const rule = (k: keyof typeof rules) => (e: React.ChangeEvent<HTMLInputElement>) => setRules({ ...rules, [k]: e.target.checked })
  return (
    <form className="space-y-5" onSubmit={(e) => { e.preventDefault(); if (!problem) save.mutate() }}>
      <Checkbox label={tt('servers.monitoringTab.sendAlertsServer')} checked={enabled} onChange={(e) => setEnabled(e.target.checked)} />
      <fieldset disabled={!enabled} className="space-y-4 disabled:opacity-60">
        <legend className="sr-only">{tt('servers.monitoringTab.whatAlertAbout')}</legend>
        <Checkbox label={tt('servers.monitoringTab.serverStoppedWithoutBeing')} hint={tt('servers.monitoringTab.stoppingFromKoraDoes')} checked={rules.stopped} onChange={rule('stopped')} />
        <Checkbox label={tt('servers.monitoringTab.serverCameBackUp')} checked={rules.recovered} onChange={rule('recovered')} />
        <Checkbox label={tt('servers.monitoringTab.diskSpaceLow')} checked={rules.disk} onChange={rule('disk')} />
        <div className="grid gap-4 pl-7 sm:grid-cols-2">
          <Field label={tt('servers.monitoringTab.alertBelowGbFree')}><Input inputMode="decimal" value={disk} onChange={(e) => setDisk(e.target.value)} /></Field>
          <Field label={tt('servers.monitoringTab.belowFree')}><Input inputMode="decimal" value={diskPct} onChange={(e) => setDiskPct(e.target.value)} /></Field>
        </div>
        <Checkbox label={tt('servers.monitoringTab.memoryNearlyFull')} checked={rules.ram} onChange={rule('ram')} />
        <div className="pl-7 sm:max-w-xs"><Field label={tt('servers.monitoringTab.alertAboveMachineUse')}><Input inputMode="decimal" value={ram} onChange={(e) => setRam(e.target.value)} /></Field></div>
        <Checkbox label={tt('servers.monitoringTab.updatesNeedAttention')} hint={tt('servers.monitoringTab.someResourcesChangesKora')} checked={rules.updates} onChange={rule('updates')} />
      </fieldset>
      <div className="space-y-3 border-t border-border pt-4">
        <h3 className="text-sm font-semibold">{tt('servers.monitoringTab.whereAlertsSent')}</h3>
        <p className="text-sm text-muted-foreground">{tt('servers.monitoringTab.alertsAlwaysShowTop')}</p>
        <Checkbox label="Discord" hint={tt('servers.monitoringTab.usesWebhookNotificationsTab')} checked={discord} onChange={(e) => setDiscord(e.target.checked)} />
        <div className="space-y-2">
          {cur.webhook.configured && (
            <div className="text-sm">{rich('servers.monitoringTab.webAddressCodeNode', { code: <code className="break-all rounded bg-ink px-2 py-1 font-mono text-xs">{cur.webhook.redacted}</code> })}</div>
          )}
          <Field label={cur.webhook.configured ? tt('servers.monitoringTab.replaceWebAddress') : tt('servers.monitoringTab.webAddressCallOptional')} hint={tt('servers.monitoringTab.koraSendsAlertAddress')}>
            <Input type="password" autoComplete="off" value={url} onChange={(e) => setUrl(e.target.value)} placeholder={tt('servers.monitoringTab.https')} />
          </Field>
          {cur.webhook.configured && <Checkbox label={tt('servers.monitoringTab.removeSavedWebAddress')} checked={removeUrl} onChange={(e) => setRemoveUrl(e.target.checked)} />}
        </div>
        <p className="text-xs text-muted-foreground">{tt('servers.monitoringTab.emailAlertsAvailableYet')}</p>
      </div>
      {problem && <p role="alert" className="text-sm text-danger-text">{problem}</p>}
      <Button type="submit" variant="default" disabled={!!problem || save.isPending}>{tt('servers.monitoringTab.saveAlertSettings')}</Button>
      {save.isSuccess && <Alert tone="success">{tt('servers.monitoringTab.alertSettingsSaved')}</Alert>}
      <ErrorNotice error={save.error} />
    </form>
  )
}

export function MonitoringTab({ server: s }: { server: ServerDetail }) {
  const [range, setRange] = useState<MetricsRange>('1h')
  const blocked = whyNot(useRole(), 'admin')
  // `can_restart` is only true when this panel is the installed background service, so it keeps running by itself.
  const sys = useQuery({ queryKey: ['system'], queryFn: api.system, staleTime: 60_000 })
  const settings = useQuery({ queryKey: ['alert-settings', s.name], queryFn: () => api.alerts.settings(s.name), enabled: !blocked })
  return (
    <div className="space-y-5">
      <Card>
        <CardHeader className="flex-row flex-wrap items-center justify-between gap-3">
          <CardTitle>{tt('servers.monitoringTab.health')}</CardTitle>
          <div role="group" aria-label={tt('servers.monitoringTab.timeRange')} className="inline-flex overflow-hidden rounded-md border border-border">
            {RANGES.map((r) => (
              <button key={r.id} type="button" aria-pressed={range === r.id} onClick={() => setRange(r.id)} className={cn('px-3 py-1.5 text-xs font-medium hover:bg-accent', range === r.id && 'bg-accent text-foreground')}>
                {tt(r.label)}
              </button>
            ))}
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          <Charts name={s.name} range={range} />
          {sys.data?.can_restart
            ? <p className="text-xs text-muted-foreground">{tt('servers.monitoringTab.koraPanelRunsBackground')}</p>
            : <p className="text-xs text-muted-foreground">{rich('servers.monitoringTab.koraOnlyMeasuresWhile', { code: (c) => <code>{c}</code> })}</p>}
        </CardContent>
      </Card>
      <Card className="max-w-3xl">
        <CardHeader><CardTitle>{tt('servers.monitoringTab.alerts')}</CardTitle></CardHeader>
        <CardContent>
          {blocked && <Alert tone="info" title={tt('servers.monitoringTab.alertSettingsAvailable')}>{blocked}</Alert>}
          {settings.isLoading && <Skeleton className="h-48" />}
          {settings.isError && <ErrorNotice error={settings.error} />}
          {settings.data && <SettingsForm key={s.name} name={s.name} cur={settings.data} />}
        </CardContent>
      </Card>
    </div>
  )
}
