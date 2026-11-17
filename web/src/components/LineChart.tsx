import { cn } from '@/lib/utils'
import { t as tt } from '@/i18n'
import '@/i18n/en/core'

export interface ChartPoint {
  t: number
  v: number | null
}

const W = 600
const H = 120

/** Split into runs without gaps so a stopped server draws a break instead of a false line. */
function runs(points: ChartPoint[]): ChartPoint[][] {
  const out: ChartPoint[][] = []
  let cur: ChartPoint[] = []
  for (const p of points) {
    if (p.v === null) {
      if (cur.length) out.push(cur)
      cur = []
    } else cur.push(p)
  }
  if (cur.length) out.push(cur)
  return out
}

export interface LineChartProps {
  title: string
  points: ChartPoint[]
  /** Fixed top of the scale (for example 100 for percent). Otherwise the highest value, rounded up. */
  max?: number
  format: (v: number) => string
  /** Text colour class: the line and the area use `currentColor`. */
  tone?: string
  className?: string
}

/** A small inline SVG line chart: no library, no lime (the one lime element of a view is its main button). */
export function LineChart({ title, points, max, format, tone = 'text-info', className }: LineChartProps) {
  const vals = points.flatMap((p) => (p.v === null ? [] : [p.v]))
  const top = max ?? Math.max(1, niceCeil(Math.max(0, ...vals)))
  const t0 = points[0]?.t ?? 0
  const t1 = points[points.length - 1]?.t ?? 1
  const span = Math.max(1, t1 - t0)
  const x = (t: number) => ((t - t0) / span) * W
  const y = (v: number) => H - Math.min(1, v / top) * (H - 6) - 3
  const last = [...points].reverse().find((p) => p.v !== null)?.v ?? null
  const peak = vals.length ? Math.max(...vals) : null
  const label = vals.length ? tt('core.lineChart.titleNowFormatHighest', { title, format: format(last ?? 0), format2: format(peak ?? 0) }) : tt('core.lineChart.titleNoMeasurementsYet', { title })
  return (
    <figure className={cn('space-y-1', className)}>
      <figcaption className="flex items-baseline justify-between text-sm">
        <span className="font-medium">{title}</span>
        <span className="text-xs text-muted-foreground">{vals.length ? tt('core.lineChart.nowFormatHighestFormat', { format: format(last ?? 0), format2: format(peak ?? 0) }) : tt('core.lineChart.noData')}</span>
      </figcaption>
      <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label={label} className={cn('h-28 w-full rounded-md border border-border bg-ink', tone)} preserveAspectRatio="none">
        {[0.25, 0.5, 0.75].map((f) => (
          <line key={f} x1={0} x2={W} y1={H - f * (H - 6) - 3} y2={H - f * (H - 6) - 3} stroke="currentColor" strokeOpacity={0.12} vectorEffect="non-scaling-stroke" />
        ))}
        {runs(points).map((run, i) =>
          run.length === 1 ? (
            <circle key={i} cx={x(run[0].t)} cy={y(run[0].v ?? 0)} r={2} fill="currentColor" />
          ) : (
            <g key={i}>
              <polygon points={`${x(run[0].t)},${H} ${run.map((p) => `${x(p.t)},${y(p.v ?? 0)}`).join(' ')} ${x(run[run.length - 1].t)},${H}`} fill="currentColor" fillOpacity={0.12} />
              <polyline points={run.map((p) => `${x(p.t)},${y(p.v ?? 0)}`).join(' ')} fill="none" stroke="currentColor" strokeWidth={2} strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
            </g>
          ),
        )}
      </svg>
      <div className="flex justify-between text-[11px] text-muted-foreground">
        <span>{points.length ? new Date(t0 * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : ''}</span>
        <span>{tt('core.lineChart.topScaleFormat', { format: format(top) })}</span>
        <span>{points.length ? new Date(t1 * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : ''}</span>
      </div>
    </figure>
  )
}

/** 7 → 8, 23 → 25, 140 → 150: a tidy top for the scale. */
export function niceCeil(v: number): number {
  if (v <= 0) return 1
  const mag = 10 ** Math.floor(Math.log10(v))
  for (const m of [1, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10]) if (v <= m * mag) return m * mag
  return 10 * mag
}
