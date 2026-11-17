/**
 * The message syntax (a small subset of ICU MessageFormat) and how a parsed message is turned into text.
 * No dependencies, no React: the platform's `Intl` does plurals, numbers, dates and lists.
 *
 *   Plain text              Your servers
 *   An argument             Hello {name}
 *   Formatted argument      {n, number}  {when, date}  {when, time}  {when, datetime}  {when, relative}  {items, list}
 *                           {bytes, bytes}  {seconds, duration}
 *   A piece of the page     {icon, node}   (an element the code passes in, drawn where the translation puts it)
 *   Plural                  {n, plural, =0 {no files} one {# file} other {# files}}   (# is the number, formatted)
 *   Select                  {kind, select, branch {On a branch} other {Somewhere else}}
 *   Tags (rich text)        Read <link>the guide</link> or run <code>{cmd}</code>
 *   Escapes                 \{ \} \< \> \# \\   (an apostrophe is just an apostrophe: "Kora's")
 *
 * Tags are filled with functions by `rich()` (React): the translation decides where the link goes in the sentence,
 * the code decides what a link is. There is no HTML anywhere, so nothing is ever injected.
 */

export type Node =
  | string
  | { k: 'arg'; name: string; fmt?: Fmt }
  | { k: 'hash' }
  | { k: 'plural'; name: string; offset: number; cases: Record<string, Node[]> }
  | { k: 'select'; name: string; cases: Record<string, Node[]> }
  | { k: 'tag'; name: string; children: Node[] }

export type Fmt = 'number' | 'date' | 'time' | 'datetime' | 'relative' | 'list' | 'bytes' | 'duration' | 'node'
const FMTS: readonly string[] = ['number', 'date', 'time', 'datetime', 'relative', 'list', 'bytes', 'duration', 'node']

export class MessageSyntaxError extends Error {}

/** Parse one message. Throws `MessageSyntaxError` with the position, so a broken catalogue entry fails its test. */
export function parse(src: string): Node[] {
  let i = 0
  const fail = (why: string): never => {
    throw new MessageSyntaxError(`${why} at ${i} in "${src}"`)
  }

  const ident = (): string => {
    const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(src.slice(i))
    if (!m) return fail('expected a name')
    i += m[0].length
    return m[0]
  }
  const skipSpace = () => {
    while (i < src.length && /\s/.test(src[i]!)) i++
  }

  /** Nodes until `stop` (a closing brace of the branch, or a closing tag, or the end). */
  const nodes = (inPlural: boolean, closing: string | null): Node[] => {
    const out: Node[] = []
    let text = ''
    const flush = () => {
      if (text) out.push(text)
      text = ''
    }
    while (i < src.length) {
      const c = src[i]!
      if (c === '\\') {
        const n = src[i + 1]
        if (n === undefined) fail('a backslash at the end')
        text += n
        i += 2
      } else if (c === '}') {
        if (closing === '}') break
        fail('an unexpected }')
      } else if (c === '{') {
        flush()
        i++
        out.push(argument())
      } else if (c === '<') {
        if (src[i + 1] === '/') {
          if (closing !== null && closing !== '}' && src.startsWith(`</${closing}>`, i)) break
          fail('a closing tag that was not opened')
        }
        const m = /^<([A-Za-z][A-Za-z0-9]*)>/.exec(src.slice(i))
        if (!m) {
          // A lone "<" is just a character ("a < b"): tags are letters straight after it.
          text += c
          i++
          continue
        }
        flush()
        i += m[0].length
        const name = m[1]!
        const children = nodes(inPlural, name)
        if (!src.startsWith(`</${name}>`, i)) fail(`<${name}> is not closed`)
        i += name.length + 3
        out.push({ k: 'tag', name, children })
      } else if (c === '#' && inPlural) {
        flush()
        out.push({ k: 'hash' })
        i++
      } else {
        text += c
        i++
      }
    }
    flush()
    return out
  }

  /** After a "{": `name}`, `name, fmt}`, `name, plural, ...}` or `name, select, ...}`. */
  const argument = (): Node => {
    skipSpace()
    const name = ident()
    skipSpace()
    if (src[i] === '}') {
      i++
      return { k: 'arg', name }
    }
    if (src[i] !== ',') fail('expected , or }')
    i++
    skipSpace()
    const kind = ident()
    skipSpace()
    if (kind === 'plural' || kind === 'select') {
      if (src[i] !== ',') fail(`expected , after ${kind}`)
      i++
      let offset = 0
      skipSpace()
      const off = /^offset:(\d+)/.exec(src.slice(i))
      if (off && kind === 'plural') {
        offset = Number(off[1])
        i += off[0].length
        skipSpace()
      }
      const cases: Record<string, Node[]> = {}
      while (i < src.length && src[i] !== '}') {
        const m = /^(=\d+|[A-Za-z_][A-Za-z0-9_-]*)/.exec(src.slice(i))
        if (!m) return fail('expected a case name')
        i += m[0].length
        skipSpace()
        if (src[i] !== '{') fail(`expected { after ${m[0]}`)
        i++
        cases[m[0]] = nodes(kind === 'plural', '}')
        if (src[i] !== '}') fail('a case is not closed')
        i++
        skipSpace()
      }
      if (src[i] !== '}') fail(`${kind} is not closed`)
      i++
      if (!('other' in cases)) fail(`${kind} needs an "other" case`)
      return kind === 'plural' ? { k: 'plural', name, offset, cases } : { k: 'select', name, cases }
    }
    if (!FMTS.includes(kind)) fail(`unknown format "${kind}"`)
    if (src[i] !== '}') fail('expected }')
    i++
    return { k: 'arg', name, fmt: kind as Fmt }
  }

  const out = nodes(false, null)
  if (i < src.length) fail('trailing text')
  return out
}

/** Every argument a message needs: its name and what kind of value it takes. Used for the generated types and the tests. */
export type ArgKind = 'any' | 'number' | 'date' | 'list' | 'select' | 'tag' | 'node'
export function args(ns: Node[], into: Map<string, ArgKind> = new Map()): Map<string, ArgKind> {
  for (const n of ns) {
    if (typeof n === 'string' || n.k === 'hash') continue
    if (n.k === 'arg') {
      const kind: ArgKind = !n.fmt ? 'any' : n.fmt === 'node' ? 'node' : n.fmt === 'list' ? 'list' : n.fmt === 'date' || n.fmt === 'time' || n.fmt === 'datetime' || n.fmt === 'relative' ? 'date' : 'number'
      into.set(n.name, merge(into.get(n.name), kind))
    } else if (n.k === 'tag') {
      into.set(n.name, 'tag')
      args(n.children, into)
    } else {
      into.set(n.name, merge(into.get(n.name), n.k === 'plural' ? 'number' : 'select'))
      for (const c of Object.values(n.cases)) args(c, into)
    }
  }
  return into
}
const merge = (a: ArgKind | undefined, b: ArgKind): ArgKind => (a === undefined || a === 'any' ? b : a)

// ---- formatting ---------------------------------------------------------------------------------------------------

export type Value = string | number | boolean | Date | string[] | ((children: unknown[]) => unknown) | object | null | undefined

/** What a message is formatted with. `intl` is the locale's formatters (see format.ts). */
export interface Context {
  locale: string
  params: Record<string, Value>
  intl: {
    number(n: number): string
    date(d: Date | number, style: 'date' | 'time' | 'datetime'): string
    relative(d: Date | number): string
    list(items: string[]): string
    bytes(n: number): string
    duration(seconds: number): string
    plural(n: number): string
  }
  /** Called for a problem in the message (a missing argument). */
  warn?: (text: string) => void
}

/** The pieces of a formatted message: strings, and whatever the tag functions returned. */
export function format(ns: Node[], ctx: Context, hash?: string): unknown[] {
  const out: unknown[] = []
  const push = (v: unknown) => {
    if (typeof v === 'string' && typeof out[out.length - 1] === 'string') out[out.length - 1] += v
    else out.push(v)
  }
  for (const n of ns) {
    if (typeof n === 'string') push(n)
    else if (n.k === 'hash') push(hash ?? '#')
    else if (n.k === 'arg') {
      const v = ctx.params[n.name]
      if (v === undefined || v === null) {
        ctx.warn?.(`missing argument "${n.name}"`)
        push(`{${n.name}}`)
      } else push(n.fmt === 'node' ? v : scalar(n.fmt, v, ctx))
    } else if (n.k === 'tag') {
      const inner = format(n.children, ctx, hash)
      const fn = ctx.params[n.name]
      if (typeof fn === 'function') push(fn(inner))
      else {
        ctx.warn?.(`missing tag function "${n.name}"`)
        for (const p of inner) push(p)
      }
    } else if (n.k === 'plural') {
      const v = ctx.params[n.name]
      const num = typeof v === 'number' ? v : Number(v)
      if (Number.isNaN(num)) ctx.warn?.(`"${n.name}" is not a number`)
      const exact = n.cases[`=${num}`]
      const branch = exact ?? n.cases[ctx.intl.plural(num - n.offset)] ?? n.cases.other!
      for (const p of format(branch, ctx, ctx.intl.number(num - n.offset))) push(p)
    } else {
      const v = String(ctx.params[n.name] ?? '')
      for (const p of format(n.cases[v] ?? n.cases.other!, ctx, hash)) push(p)
    }
  }
  return out
}

function scalar(fmt: Fmt | undefined, v: Value, ctx: Context): string {
  switch (fmt) {
    case 'number':
      return ctx.intl.number(Number(v))
    case 'bytes':
      return ctx.intl.bytes(Number(v))
    case 'duration':
      return ctx.intl.duration(Number(v))
    case 'date':
    case 'time':
    case 'datetime':
      return ctx.intl.date(v as Date | number, fmt)
    case 'relative':
      return ctx.intl.relative(v as Date | number)
    case 'list':
      return ctx.intl.list(Array.isArray(v) ? v : [String(v)])
    default:
      return String(v)
  }
}

/** Map the literal text of a parsed message (not the arguments or tags). Used for the pseudo-language. */
export function mapText(ns: Node[], f: (s: string) => string): Node[] {
  return ns.map((n): Node => {
    if (typeof n === 'string') return f(n)
    if (n.k === 'tag') return { ...n, children: mapText(n.children, f) }
    if (n.k === 'plural' || n.k === 'select') return { ...n, cases: Object.fromEntries(Object.entries(n.cases).map(([k, v]) => [k, mapText(v, f)])) }
    return n
  })
}
