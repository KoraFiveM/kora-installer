/**
 * Translations: `t('area.id', { params })`. See docs/translations.md for the whole picture.
 *
 *  - English (the catalogue in ./en/<area>.ts) is the source of truth. A file that uses ids of an area imports that
 *    area's module (`import '@/i18n/en/servers'`), which registers it: the strings travel in the same chunk as the
 *    page that uses them, so the first load carries only what the first screen needs.
 *  - Other languages are one lazily loaded file each (./de/index.ts). A missing id falls back to English.
 *  - `t` reads the current language from module state; the app re-renders from the top when it changes (App.tsx).
 *    Do not call `t` while a module is being loaded (top-level constants): it would be fixed to the language at that
 *    moment. Keep ids in the constant and call `t` where the text is shown.
 */
import { args, format, mapText, parse, type Context, type Node, type Value } from './icu'
import { intlFor, intlLocale } from './format'
import { DEFAULT_LOCALE, STORAGE_KEY, detect, infoOf, offered } from './locales'
import { pseudo } from './pseudo'
import type { MessageId, MessageParams, PlainId, RichId } from './types'

export type { MessageId, MessageParams, PlainId, RichId } from './types'
export { LOCALES, offered, infoOf, DEFAULT_LOCALE } from './locales'
export { intlFor, clock } from './format'

/**
 * Numbers, sizes, dates and times for the current language, through `Intl`. Use these instead of `toFixed`, `toLocaleString`
 * or hand-made "5 minutes ago" text. Unix seconds are what the server sends: `unix()` turns them into a Date.
 */
export const fmt = {
  number: (n: number) => intlFor(current).number(n),
  /** 152 kB, 1.5 GB (the unit and the decimal mark follow the language). */
  bytes: (n: number) => intlFor(current).bytes(n),
  date: (d: Date | number) => intlFor(current).date(d, 'date'),
  time: (d: Date | number) => intlFor(current).date(d, 'time'),
  dateTime: (d: Date | number) => intlFor(current).date(d, 'datetime'),
  /** "3 days ago", "in 5 minutes". `now` (ms) defaults to the clock. */
  relative: (d: Date | number, now?: number) => intlFor(current).relative(d, now),
  /** The name of a weekday: 0 = Monday ... 6 = Sunday. */
  weekday: (i: number) => new Intl.DateTimeFormat(intlLocale(current), { weekday: 'long', timeZone: 'UTC' }).format(new Date(Date.UTC(2024, 0, 1 + i))),
  /** "1h 5m". */
  duration: (seconds: number) => intlFor(current).duration(seconds),
  list: (items: string[]) => intlFor(current).list(items),
  /** A percentage like 12.5% (the argument is 0-100). */
  percent: (n: number, digits = 1) => new Intl.NumberFormat(intlLocale(current), { style: 'percent', maximumFractionDigits: digits }).format(n / 100),
  /** A decimal with a fixed number of digits (the language's own decimal mark). */
  fixed: (n: number, digits: number) => new Intl.NumberFormat(intlLocale(current), { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(n),
}
/** Unix seconds (what the server sends) as a Date. */
export const unix = (seconds: number): Date => new Date(seconds * 1000)

type Table = Record<string, string>

const tables = new Map<string, Table>()
const parsed = new Map<string, Node[]>()
const warned = new Set<string>()
let current = DEFAULT_LOCALE
const listeners = new Set<() => void>()

/** Development and the mock build (browser tests) have the pseudo-language and a `?lang=` switch; production does not. */
export const DEV_LOCALES: boolean = typeof __KORA_MOCK__ !== 'undefined' && __KORA_MOCK__ ? true : import.meta.env.DEV

/** Add messages of a language (called by the catalogue modules). */
export function register(locale: string, messages: Table): void {
  const t = tables.get(locale) ?? {}
  Object.assign(t, messages)
  tables.set(locale, t)
}

/** A copy of the messages registered for a language (the completeness tests and tools read this). */
export function catalogue(locale: string): Record<string, string> {
  return { ...(tables.get(locale) ?? {}) }
}

export function hasLocale(locale: string): boolean {
  return tables.has(locale)
}

function warnOnce(key: string, text: string) {
  if (!DEV_LOCALES || warned.has(key)) return
  warned.add(key)
  console.warn(`[i18n] ${text}`)
}

/** The parsed message in the current language (English when it has none; the id itself, as a last resort). */
function lookup(id: string): { nodes: Node[]; locale: string } {
  const key = `${current}\0${id}`
  const hit = parsed.get(key)
  if (hit) return { nodes: hit, locale: current }
  let src: string | undefined
  let from = current
  if (current === 'en-XA') {
    src = tables.get('en')?.[id]
  } else {
    src = tables.get(current)?.[id]
    if (src === undefined && current !== 'en') {
      src = tables.get('en')?.[id]
      from = 'en'
      if (src !== undefined) warnOnce(`${current}:${id}`, `"${id}" has no ${current} translation yet; showing English`)
    }
  }
  if (src === undefined) {
    warnOnce(`missing:${id}`, `unknown message id "${id}"`)
    const nodes: Node[] = [id]
    return { nodes, locale: from }
  }
  let nodes = parse(src)
  if (current === 'en-XA') nodes = mapText(nodes, pseudo)
  parsed.set(key, nodes)
  return { nodes, locale: current }
}

function context(params: Record<string, Value>, id: string): Context {
  return { locale: current, params, intl: intlFor(current), warn: (w) => warnOnce(`${id}:${w}`, `${id}: ${w}`) }
}

type ArgsFor<K extends MessageId> = K extends keyof MessageParams ? [params: MessageParams[K]] : []

/** The text of a message. Messages with <tags> need `rich()` (react.tsx). */
export function t<K extends PlainId>(id: K | Msg<K>, ...args: ArgsFor<K>): string {
  const key: string = typeof id === 'string' ? id : id.msgId
  const params = ((args as unknown[])[0] ?? {}) as Record<string, Value>
  return format(lookup(key).nodes, context(params, key)).join('')
}

/** A message id kept in a table (see `msg`). It is not text: pass it to `t()` where it is drawn. */
export interface Msg<K extends PlainId = PlainId> {
  readonly msgId: K
}

/**
 * Mark a message in a module-level table (`{ running: msg('jobs.state.running') }`): the id is checked against the
 * catalogue, and the value cannot be drawn by mistake (it is not a string): show it with `t(row.label)` where it is
 * drawn. Calling `t` while the module loads would fix the language at that moment.
 */
export function msg<K extends PlainId>(id: K): Msg<K> {
  return {
    msgId: id,
    // A Msg pasted into a template literal or a string sum would show "[object Object]": in development and tests say so loudly.
    toString() {
      if (DEV_LOCALES) throw new Error(`msg('${id}') used as text: wrap it in t()`)
      return id
    },
  } as Msg<K>
}

/** The pieces of a message with tags (strings and what the tag functions returned). `rich()` wraps this. */
export function pieces<K extends RichId>(id: K, params: Record<string, Value>): unknown[] {
  return format(lookup(id).nodes, context(params, id))
}

/**
 * Text the SERVER sent as a message id plus parameters (a Problem's `i18n`): the message in the current language, or null when
 * this panel has no such message (an older panel, a newer server): the caller shows the server's English text instead.
 * Only ids under `errors.problems.` are read this way, and only messages without tags; the parameters are always text.
 */
export function tById(id: string, params: Record<string, string> = {}): string | null {
  if (!id.startsWith('errors.problems.')) return null
  const en = tables.get('en')?.[id]
  if (en === undefined) return null
  const nodes = lookup(id).nodes
  const kinds = args(parse(en))
  const typed: Record<string, Value> = {}
  for (const [k, v] of Object.entries(params)) typed[k] = kinds.get(k) === 'number' ? Number(v) : v
  if ([...kinds.values()].some((k) => k === 'tag' || k === 'node')) return null
  return format(nodes, context(typed, id)).join('')
}

/** Show which server texts are not translated (an "EN" mark): a switch for people who translate, in the browser's storage. */
export function debugUntranslated(): boolean {
  try {
    return localStorage.getItem('kora.i18n.debug') === '1'
  } catch {
    return false
  }
}

export function locale(): string {
  return current
}

export function subscribe(fn: () => void): () => void {
  listeners.add(fn)
  return () => listeners.delete(fn)
}

// ---- choosing a language ------------------------------------------------------------------------------------------

/** Languages other than English are separate files, fetched when chosen. */
const loaders: Record<string, () => Promise<unknown>> = {
  de: () => import('./de'),
}

export async function load(code: string): Promise<void> {
  if (tables.has(code) || code === 'en' || code === 'en-XA') return
  const l = loaders[code]
  if (l) await l()
}

/** Show the panel in `code` (loading its messages first), remember it for this browser, and tell the page. */
export async function setLocale(code: string, remember = true): Promise<void> {
  const ok = offered(DEV_LOCALES).some((l) => l.code === code) ? code : DEFAULT_LOCALE
  try {
    await load(ok)
  } catch {
    // The file could not be fetched (offline, or the panel was updated meanwhile): stay as we are.
    return
  }
  current = ok
  parsed.clear()
  if (typeof document !== 'undefined') {
    const info = infoOf(ok)
    document.documentElement.lang = ok === 'en' ? 'en-GB' : ok === 'en-XA' ? 'en-GB-x-pseudo' : intlLocale(ok)
    document.documentElement.dir = info.dir
  }
  if (remember) {
    try {
      localStorage.setItem(STORAGE_KEY, ok)
    } catch {
      // Storage is blocked: the choice lasts until the page is closed.
    }
  }
  for (const f of listeners) f()
}

/** The language to start in: `?lang=` (development and mock builds only), then what this browser remembered, then the browser's own list. */
export function initialLocale(): string {
  if (DEV_LOCALES && typeof location !== 'undefined') {
    const q = new URLSearchParams(location.search).get('lang')
    if (q && offered(true).some((l) => l.code === q)) return q
  }
  try {
    const saved = localStorage.getItem(STORAGE_KEY)
    if (saved && offered(DEV_LOCALES).some((l) => l.code === saved)) return saved
  } catch {
    // no storage
  }
  return detect(typeof navigator !== 'undefined' ? (navigator.languages?.length ? navigator.languages : [navigator.language]) : [], DEV_LOCALES)
}

/** Called once before the first render: load and apply the starting language (not stored: only a choice in the picker is). */
export async function boot(): Promise<void> {
  await setLocale(initialLocale(), false)
}
