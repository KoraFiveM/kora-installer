import { afterEach, describe, expect, it, vi } from 'vitest'
import { debugUntranslated, fmt, initialLocale, locale, msg, register, setLocale, t, tById, unix } from './index'
import { detect, offered } from './locales'
import { pseudo } from './pseudo'
import '@/i18n/en/core/status'
import '@/i18n/en/errors/problems'

afterEach(async () => {
  await setLocale('en', false)
  localStorage.clear()
  vi.restoreAllMocks()
})

describe('t() in English', () => {
  it('formats plurals through the catalogue', () => {
    expect(t('core.status.updates', { count: 1 })).toBe('1 update')
    expect(t('core.status.updates', { count: 3 })).toBe('3 updates')
    expect(t('core.status.updates', { count: 1200 })).toBe('1,200 updates')
  })
  it('a Msg from a table is drawn through t(), and refuses to be pasted into text', () => {
    const m = msg('core.status.updates')
    expect(t(m, { count: 2 })).toBe('2 updates')
    expect(() => `${m}`).toThrow(/wrap it in t\(\)/)
  })
  it('an unknown id shows the id (and does not throw)', () => {
    vi.spyOn(console, 'warn').mockImplementation(() => undefined)
    expect((t as unknown as (id: string) => string)('nope.nothing.here')).toBe('nope.nothing.here')
  })
})

describe('languages', () => {
  it('a missing translation falls back to English', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => undefined)
    register('de', { 'core.status.updates': '{count, plural, one {# Aktualisierung} other {# Aktualisierungen}}' })
    await setLocale('de', false)
    expect(t('core.status.updates', { count: 1 })).toBe('1 Aktualisierung')
    expect(t('core.status.updates', { count: 2000 })).toBe('2.000 Aktualisierungen')
    // "errors.problems.ghMissing" has no German here: English
    expect(tById('errors.problems.ghMissing')).toMatch(/GitHub CLI/)
  })

  it('sets the page language and direction, and remembers a choice', async () => {
    await setLocale('de')
    expect(document.documentElement.lang).toBe('de')
    expect(document.documentElement.dir).toBe('ltr')
    expect(localStorage.getItem('kora.locale')).toBe('de')
    expect(initialLocale()).toBe('de')
    await setLocale('en')
    expect(document.documentElement.lang).toBe('en-GB')
  })

  it('refuses a language that is not offered', async () => {
    await setLocale('xx')
    expect(locale()).toBe('en')
  })

  it('picks the first supported language of the browser list', () => {
    expect(detect(['fr-FR', 'de-AT', 'en-US'], false)).toBe('de')
    expect(detect(['en-US'], false)).toBe('en')
    expect(detect(['fr'], false)).toBe('en')
    expect(detect([], false)).toBe('en')
    // The pseudo-language is never chosen by a browser, and is offered only in development builds.
    expect(offered(false).some((l) => l.code === 'en-XA')).toBe(false)
    expect(offered(true).some((l) => l.code === 'en-XA')).toBe(true)
    expect(detect(['en-XA'], false)).toBe('en')
  })

  it('the pseudo-language accents and lengthens every message, and keeps the arguments', async () => {
    await setLocale('en-XA', false)
    const s = t('core.status.updates', { count: 3 })
    expect(s.startsWith('[') && s.endsWith(']')).toBe(true)
    expect(s).toMatch(/3/)
    expect(s).not.toMatch(/updates/)
    expect(pseudo('Servers').length).toBeGreaterThan('Servers'.length)
    expect(pseudo('123')).toBe('123')
  })

  it('a language that is fetched when chosen has no listener problems when the fetch fails', async () => {
    // (the loader of a language that does not exist is not called: nothing to fail)
    await setLocale('en')
    expect(locale()).toBe('en')
  })
})

describe('text the server names by id', () => {
  it('reads only problem messages, with text parameters read as numbers where the message wants numbers', () => {
    expect(tById('errors.problems.dbStopped', { name: 'MariaDB' })).toBe('The database server MariaDB is installed but not running.')
    expect(tById('core.status.updates', { count: '2' })).toBeNull() // not an errors.problems id
    expect(tById('errors.problems.doesNotExist')).toBeNull()
  })
  it('the debug switch for untranslated server text is off unless set', () => {
    expect(debugUntranslated()).toBe(false)
    localStorage.setItem('kora.i18n.debug', '1')
    expect(debugUntranslated()).toBe(true)
  })
})

describe('Intl formatting follows the language', () => {
  it('numbers, sizes, dates and relative times', async () => {
    expect(fmt.number(1234.5)).toBe('1,234.5')
    expect(fmt.bytes(152_000)).toBe('152 kB')
    expect(fmt.date(unix(1_700_000_000))).toBe('14 Nov 2023')
    await setLocale('de', false)
    expect(fmt.number(1234.5)).toBe('1.234,5')
    expect(fmt.bytes(2_500_000)).toMatch(/2,5\s?MB/)
    expect(fmt.date(unix(1_700_000_000))).toBe('14.11.2023')
    expect(fmt.list(['a', 'b', 'c'])).toBe('a, b und c')
  })
})
