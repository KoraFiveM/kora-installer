import { describe, expect, it } from 'vitest'
import { args, format, MessageSyntaxError, parse, type Context, type Value } from './icu'
import { clock, intlFor } from './format'

const ctx = (params: Record<string, Value> = {}, locale = 'en'): Context => ({ locale, params, intl: intlFor(locale) })
const run = (src: string, params: Record<string, Value> = {}, locale = 'en') => format(parse(src), ctx(params, locale)).join('')

describe('messages', () => {
  it('plain text, apostrophes and escapes', () => {
    expect(run("Kora's servers")).toBe("Kora's servers")
    expect(run('Use \\{braces\\} and \\<tags\\> and 5 \\# of them')).toBe('Use {braces} and <tags> and 5 # of them')
    expect(run('a < b and b > a')).toBe('a < b and b > a')
  })

  it('arguments', () => {
    expect(run('Hello {name}!', { name: 'Sam' })).toBe('Hello Sam!')
    expect(run('{n, number} servers', { n: 1234567 })).toBe('1,234,567 servers')
    expect(run('{n, number} servers', { n: 1234567 }, 'de')).toBe('1.234.567 servers')
  })

  it('a missing argument stays visible and warns', () => {
    const w: string[] = []
    const out = format(parse('Hello {name}'), { ...ctx(), warn: (x) => w.push(x) }).join('')
    expect(out).toBe('Hello {name}')
    expect(w).toEqual(['missing argument "name"'])
  })

  describe('plural', () => {
    const m = '{n, plural, =0 {No files} one {# file} other {# files}}'
    it('English', () => {
      expect(run(m, { n: 0 })).toBe('No files')
      expect(run(m, { n: 1 })).toBe('1 file')
      expect(run(m, { n: 2 })).toBe('2 files')
      expect(run(m, { n: 1500 })).toBe('1,500 files')
    })
    it('uses the language\'s own categories (Polish has few and many)', () => {
      const pl = '{n, plural, one {# plik} few {# pliki} many {# plików} other {# pliku}}'
      expect(run(pl, { n: 1 }, 'pl')).toBe('1 plik')
      expect(run(pl, { n: 3 }, 'pl')).toBe('3 pliki')
      expect(run(pl, { n: 5 }, 'pl')).toBe('5 plików')
    })
    it('offset', () => {
      const m2 = '{n, plural, offset:1 =0 {nobody} =1 {{who}} one {{who} and one other} other {{who} and # others}}'
      expect(run(m2, { n: 0, who: 'Sam' })).toBe('nobody')
      expect(run(m2, { n: 1, who: 'Sam' })).toBe('Sam')
      expect(run(m2, { n: 2, who: 'Sam' })).toBe('Sam and one other')
      expect(run(m2, { n: 4, who: 'Sam' })).toBe('Sam and 3 others')
    })
  })

  it('select, with a fallback', () => {
    const m = '{kind, select, branch {On branch {name}} pr {Trying pull request #{name}} other {Somewhere else}}'
    expect(run(m, { kind: 'branch', name: 'fix' })).toBe('On branch fix')
    expect(run(m, { kind: 'pr', name: '12' })).toBe('Trying pull request #12')
    expect(run(m, { kind: 'detached', name: '' })).toBe('Somewhere else')
  })

  it('nested: a select inside a plural', () => {
    const m = '{n, plural, one {1 {kind, select, a {apple} other {fruit}}} other {# {kind, select, a {apples} other {fruits}}}}'
    expect(run(m, { n: 1, kind: 'a' })).toBe('1 apple')
    expect(run(m, { n: 3, kind: 'x' })).toBe('3 fruits')
  })

  it('dates, relative times, lists, sizes and durations', () => {
    expect(run('{d, date}', { d: new Date(Date.UTC(2026, 0, 5, 12)) })).toMatch(/5 Jan 2026/)
    expect(run('{d, date}', { d: new Date(Date.UTC(2026, 0, 5, 12)) }, 'de')).toMatch(/05\.01\.2026/)
    expect(run('{l, list}', { l: ['a', 'b', 'c'] })).toBe('a, b and c')
    expect(run('{l, list}', { l: ['a', 'b', 'c'] }, 'de')).toBe('a, b und c')
    expect(run('{b, bytes}', { b: 152_000 })).toMatch(/152\s?kB/)
    expect(run('{b, bytes}', { b: 2_500_000_000 }, 'de')).toMatch(/2,5\s?GB/)
    expect(run('{s, duration}', { s: 3725 })).toBe('1h 2m')
    expect(run('{s, duration}', { s: 7 })).toBe('7s')
    const real = clock.now
    clock.now = () => 1_000_000_000_000
    try {
      expect(run('{d, relative}', { d: 1_000_000_000_000 - 3 * 86_400_000 })).toBe('3 days ago')
      expect(run('{d, relative}', { d: 1_000_000_000_000 + 5 * 60_000 })).toBe('in 5 minutes')
      expect(run('{d, relative}', { d: 1_000_000_000_000 - 3 * 86_400_000 }, 'de')).toBe('vor 3 Tagen')
    } finally {
      clock.now = real
    }
  })

  it('tags call their function with the pieces between them', () => {
    const out = format(parse('Run <code>{cmd}</code> or read <link>the guide</link>.'), ctx({ cmd: 'kora panel', code: (c) => `[code:${c.join('')}]`, link: (c) => ({ link: c }) }))
    expect(out).toEqual(['Run [code:kora panel] or read ', { link: ['the guide'] }, '.'])
  })

  it('collects its arguments', () => {
    const a = args(parse('{n, plural, one {<b>{who}</b>} other {# {l, list}}} {d, date} {k, select, x {y} other {z}} {plain}'))
    expect([...a]).toEqual([['n', 'number'], ['b', 'tag'], ['who', 'any'], ['l', 'list'], ['d', 'date'], ['k', 'select'], ['plain', 'any']])
  })

  it('rejects broken messages with a position', () => {
    for (const bad of ['{name', '{n, plural, one {x}}', '{n, bogus}', 'a } b', '<b>open', '</b>', '{n, plural, other {x}', 'trailing \\']) {
      expect(() => parse(bad), bad).toThrow(MessageSyntaxError)
    }
  })
})
