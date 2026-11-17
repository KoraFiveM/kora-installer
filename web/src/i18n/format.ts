import type { Context } from './icu'

/**
 * Numbers, sizes, dates, durations, relative times and lists for one locale, all through the platform's `Intl`:
 * nothing here knows what "3 days ago" looks like in any language. The formatters are made once per locale.
 */
export type Intls = Context['intl']

const cache = new Map<string, Intls>()

/** "en" is shown the British way (the product's spelling and date order); every other tag goes to Intl as it is. */
export function intlLocale(code: string): string {
  return code === 'en' ? 'en-GB' : code === 'en-XA' ? 'en-GB' : code
}

const SECOND = 1000
const UNITS: Array<[Intl.RelativeTimeFormatUnit, number]> = [
  ['year', 365 * 24 * 3600 * SECOND],
  ['month', 30 * 24 * 3600 * SECOND],
  ['day', 24 * 3600 * SECOND],
  ['hour', 3600 * SECOND],
  ['minute', 60 * SECOND],
  ['second', SECOND],
]

/** Seconds since `Date.now()` can be swapped in tests: relative times take the moment to compare with. */
export const clock = { now: () => Date.now() }

export function intlFor(code: string): Intls {
  const hit = cache.get(code)
  if (hit) return hit
  const loc = intlLocale(code)
  const plural = new Intl.PluralRules(loc)
  const num = new Intl.NumberFormat(loc)
  const one = new Intl.NumberFormat(loc, { maximumFractionDigits: 1 })
  const rel = new Intl.RelativeTimeFormat(loc, { numeric: 'always' })
  const list = new Intl.ListFormat(loc, { style: 'long', type: 'conjunction' })
  const dateOnly = new Intl.DateTimeFormat(loc, { dateStyle: 'medium' })
  const timeOnly = new Intl.DateTimeFormat(loc, { timeStyle: 'short' })
  const both = new Intl.DateTimeFormat(loc, { dateStyle: 'medium', timeStyle: 'short' })
  const unitFmt = (unit: string, digits = 1) => new Intl.NumberFormat(loc, { style: 'unit', unit, unitDisplay: 'short', maximumFractionDigits: digits })
  const B = unitFmt('byte', 0)
  const KB = unitFmt('kilobyte', 0)
  const MB = unitFmt('megabyte')
  const GB = unitFmt('gigabyte')
  const TB = unitFmt('terabyte')
  const dur = (unit: string) => new Intl.NumberFormat(loc, { style: 'unit', unit, unitDisplay: 'narrow' })
  const intl: Intls = {
    plural: (n) => plural.select(n),
    number: (n) => (Number.isInteger(n) ? num.format(n) : one.format(n)),
    bytes: (n) => (n >= 1e12 ? TB.format(n / 1e12) : n >= 1e9 ? GB.format(n / 1e9) : n >= 1e6 ? MB.format(n / 1e6) : n >= 1e3 ? KB.format(Math.round(n / 1e3)) : B.format(n)),
    date: (d, style) => (style === 'date' ? dateOnly : style === 'time' ? timeOnly : both).format(d),
    list: (items) => list.format(items),
    relative: (d) => {
      const diff = (typeof d === 'number' ? d : d.getTime()) - clock.now()
      const abs = Math.abs(diff)
      for (const [unit, ms] of UNITS) {
        if (abs >= ms || unit === 'second') return rel.format(Math.round(diff / ms), unit)
      }
      return rel.format(0, 'second')
    },
    // "1h 5m 3s": the two largest non-zero units are enough for "how long did this take".
    duration: (s) => {
      const total = Math.max(0, Math.round(s))
      const parts: string[] = []
      const h = Math.floor(total / 3600)
      const m = Math.floor((total % 3600) / 60)
      const sec = total % 60
      if (h) parts.push(dur('hour').format(h))
      if (m) parts.push(dur('minute').format(m))
      if (sec || parts.length === 0) parts.push(dur('second').format(sec))
      return parts.slice(0, 2).join(' ')
    },
  }
  cache.set(code, intl)
  return intl
}
