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

export const METRICS_POLL_MS = 15000
const RANGES: { id: MetricsRange; label: string }[] = [
  { id: '1h', label: 'Last hour' },
  { id: '6h', label: 'Last 6 hours' },
  { id: '24h', label: 'Last 24 hours' },
]

const gb = (mb: number) => `${(mb / 1024).toFixed(1)} GB`
const mbText = (mb: number) => (mb >= 1024 ? gb(mb) : `${Math.round(mb)} MB`)

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
      <Stat label="Players" value={s?.players === undefined ? dash : String(s.players)} hint={s?.max_players ? `of ${s.max_players}` : undefined} />
      <Stat label="Server CPU" value={s?.cpu === undefined ? dash : `${s.cpu.toFixed(1)}%`} hint="of the whole machine" />
      <Stat label="Server memory" value={s?.ram_mb === undefined ? dash : mbText(s.ram_mb)} hint={s?.sys_ram_pct === undefined ? undefined : `${Math.round(s.sys_ram_pct)}% of the machine in use`} />
      <Stat label="Free disk space" value={s?.disk_free_mb === undefined ? dash : gb(s.disk_free_mb)} hint={s?.disk_free_pct === undefined ? undefined : `${Math.round(s.disk_free_pct)}% free`} />
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
        <Badge tone={latest?.up ? 'success' : 'danger'} dot>{latest ? (latest.up ? 'Running' : 'Stopped') : 'No data yet'}</Badge>
        {latest && <Badge tone={latest.game_up ? 'success' : 'neutral'} dot>{latest.game_up ? 'Game port listening' : 'Game port closed'}</Badge>}
        {latest && <Badge tone={latest.txadmin_up ? 'success' : 'neutral'} dot>{latest.txadmin_up ? 'txAdmin answering' : 'txAdmin not answering'}</Badge>}
      </div>
      <Current s={latest} />
      {samples.length === 0 && <p className="text-sm text-muted-foreground">Nothing has been measured yet. Kora looks at your servers every 15 seconds while the panel is running.</p>}
      <div className="grid gap-5 lg:grid-cols-2">
        <LineChart title="Players" points={series((s) => s.players)} format={(v) => String(Math.round(v))} tone="text-info" />
        <LineChart title="Server CPU (percent of the machine)" points={series((s) => s.cpu)} max={100} format={(v) => `${v.toFixed(0)}%`} tone="text-amber" />
        <LineChart title="Server memory" points={series((s) => s.ram_mb)} format={(v) => mbText(v)} tone="text-success" className="lg:col-span-2" />
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
    !(num(disk) >= 0 && num(disk) <= 100000) ? 'Free disk space must be a number of gigabytes, 0 or more.'
    : !(num(diskPct) >= 0 && num(diskPct) <= 100) ? 'The disk percentage must be between 0 and 100.'
    : !(num(ram) >= 1 && num(ram) <= 100) ? 'The memory limit must be between 1 and 100 percent.'
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
      <Checkbox label="Send alerts for this server" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} />
      <fieldset disabled={!enabled} className="space-y-4 disabled:opacity-60">
        <legend className="sr-only">What to alert about</legend>
        <Checkbox label="The server stopped without being asked to" hint="Stopping it from Kora does not count." checked={rules.stopped} onChange={rule('stopped')} />
        <Checkbox label="The server came back up" checked={rules.recovered} onChange={rule('recovered')} />
        <Checkbox label="Disk space is low" checked={rules.disk} onChange={rule('disk')} />
        <div className="grid gap-4 pl-7 sm:grid-cols-2">
          <Field label="Alert below (GB free)"><Input inputMode="decimal" value={disk} onChange={(e) => setDisk(e.target.value)} /></Field>
          <Field label="or below (% free)"><Input inputMode="decimal" value={diskPct} onChange={(e) => setDiskPct(e.target.value)} /></Field>
        </div>
        <Checkbox label="Memory is nearly full" checked={rules.ram} onChange={rule('ram')} />
        <div className="pl-7 sm:max-w-xs"><Field label="Alert above (% of the machine in use)"><Input inputMode="decimal" value={ram} onChange={(e) => setRam(e.target.value)} /></Field></div>
        <Checkbox label="Updates need attention" hint="Some resources have changes Kora cannot update on its own." checked={rules.updates} onChange={rule('updates')} />
      </fieldset>
      <div className="space-y-3 border-t border-border pt-4">
        <h3 className="text-sm font-semibold">Where alerts are sent</h3>
        <p className="text-sm text-muted-foreground">Alerts always show at the top of this page. You can also send them elsewhere.</p>
        <Checkbox label="Discord" hint="Uses the webhook on the Notifications tab, if one is set." checked={discord} onChange={(e) => setDiscord(e.target.checked)} />
        <div className="space-y-2">
          {cur.webhook.configured && (
            <div className="text-sm">Web address: <code className="break-all rounded bg-ink px-2 py-1 font-mono text-xs">{cur.webhook.redacted}</code></div>
          )}
          <Field label={cur.webhook.configured ? 'Replace the web address' : 'Web address to call (optional)'} hint="Kora sends the alert to this address as JSON. It is kept secret and never shown in full again.">
            <Input type="password" autoComplete="off" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://..." />
          </Field>
          {cur.webhook.configured && <Checkbox label="Remove the saved web address" checked={removeUrl} onChange={(e) => setRemoveUrl(e.target.checked)} />}
        </div>
        <p className="text-xs text-muted-foreground">Email alerts are not available yet.</p>
      </div>
      {problem && <p role="alert" className="text-sm text-destructive">{problem}</p>}
      <Button type="submit" variant="default" disabled={!!problem || save.isPending}>Save alert settings</Button>
      {save.isSuccess && <Alert tone="success">Alert settings saved.</Alert>}
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
          <CardTitle>Health</CardTitle>
          <div role="group" aria-label="Time range" className="inline-flex overflow-hidden rounded-md border border-border">
            {RANGES.map((r) => (
              <button key={r.id} type="button" aria-pressed={range === r.id} onClick={() => setRange(r.id)} className={cn('px-3 py-1.5 text-xs font-medium hover:bg-accent', range === r.id && 'bg-accent text-foreground')}>
                {r.label}
              </button>
            ))}
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          <Charts name={s.name} range={range} />
          {sys.data?.can_restart
            ? <p className="text-xs text-muted-foreground">Kora Panel runs in the background on this machine, so it keeps measuring when this page is closed.</p>
            : <p className="text-xs text-muted-foreground">Kora only measures while the panel is running. To keep watching when this page is closed, run <code>kora panel --install-service</code> on the machine.</p>}
        </CardContent>
      </Card>
      <Card className="max-w-3xl">
        <CardHeader><CardTitle>Alerts</CardTitle></CardHeader>
        <CardContent>
          {blocked && <Alert tone="info" title="Alert settings are not available to you">{blocked}</Alert>}
          {settings.isLoading && <Skeleton className="h-48" />}
          {settings.isError && <ErrorNotice error={settings.error} />}
          {settings.data && <SettingsForm key={s.name} name={s.name} cur={settings.data} />}
        </CardContent>
      </Card>
    </div>
  )
}
