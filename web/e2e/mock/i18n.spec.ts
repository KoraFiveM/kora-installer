// Languages in the browser: the picker (Settings and the sign-in screen), German (beta) on the main pages, the language a
// browser asks for, and the pseudo-language (accented, longer English) that catches text that was never put in the
// catalogue and layouts that break with long words. docs/translations.md.
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import type { Page } from '@playwright/test'
import { test, expect } from '../fixtures'
import { pseudo } from '../../src/i18n/pseudo'

// About signed-out and key-less states, not the setup checklist: start with the setup undone.
test.use({ setupState: 'pending' })

/**
 * What the mock backend says in English: server names, descriptions, alert and system-check texts. That is the SERVER's text,
 * not the panel's (docs/translations.md, "text from the server"): it is not a hard-coded string of the page.
 */
const MOCK_TEXT: string[] = (() => {
  const dir = join(process.cwd(), 'src/api/mock')
  const out: string[] = []
  for (const f of readdirSync(dir).filter((x) => x.endsWith('.ts'))) {
    for (const m of readFileSync(join(dir, f), 'utf8').matchAll(/'((?:[^'\\\n]|\\.)+)'|"((?:[^"\\\n]|\\.)+)"|`((?:[^`\\]|\\.)+)`/g)) {
      const t = (m[1] ?? m[2] ?? m[3] ?? '').replace(/\\(.)/g, '$1').replace(/\$\{[^}]*\}/g, '\uE000').replace(/\s+/g, ' ').trim()
      if (/[A-Za-z]{3,}\s+[A-Za-z]{3,}/.test(t.replace(/\uE000/g, ' '))) out.push(t)
    }
  }
  return out
})()

/** Visible text of the page that still reads as plain English: two plain-ASCII words in a row, outside code and data. */
async function plainEnglish(page: Page): Promise<string[]> {
  const found = await page.evaluate(() => {
    const out: string[] = []
    const skip = 'code, pre, kbd, samp, input, textarea, select, script, style, svg, [data-i18n-skip], #kora-mock-switcher, .font-mono'
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT)
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const el = n.parentElement
      if (!el || el.closest(skip)) continue
      const text = (n.textContent ?? '').replace(/\s+/g, ' ').trim()
      // Two ASCII words of 3+ letters in a row: the pseudo-language turns every letter into an accented one.
      // Intl writes dates and "5 hours ago" in the page language (the pseudo-language has no Intl data of its own).
      const plain = text.replace(/\d+ (second|minute|hour|day|month|year)s? ago|in \d+ (second|minute|hour|day|month|year)s?|\d{1,2} [A-Z][a-z]{2} \d{4}/g, ' ')
      if (/\b[A-Za-z]{3,}\s+[A-Za-z]{3,}\b/.test(plain)) out.push(text.slice(0, 160))
    }
    return out
  })
  // A mock sentence with a hole (`There is no server called "${name}".`) matches as a pattern.
  const patterns = MOCK_TEXT.filter((m) => m.includes('\uE000')).map((m) => new RegExp(m.split('\uE000').map((x) => x.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('.+')))
  const dynamic = [/^Installed [\d.]+ \(/] // mock sentences whose holes hold quotes (not found by the extraction above)
  return found.filter((t) => !dynamic.some((re) => re.test(t)) && !MOCK_TEXT.some((m) => (t.length >= 14 ? m.includes(t) || t.includes(m) : m === t)) && !patterns.some((re) => re.test(t)))
}

test.describe('the language picker', () => {
  test('Settings: pick German, the page changes at once, the choice is remembered and <html lang> follows', async ({ page, open, scan }) => {
    await open('/settings')
    await expect(page.getByRole('heading', { name: 'Settings', level: 1 })).toBeVisible()
    const picker = page.getByLabel('Language')
    await expect(picker).toHaveValue('en')
    // German is marked as a beta, and the pseudo-language is offered in this (mock) build only.
    await expect(picker.locator('option', { hasText: 'Deutsch (beta)' })).toHaveCount(1)
    await picker.selectOption('de')
    await expect(page.getByRole('heading', { name: 'Einstellungen', level: 1 })).toBeVisible()
    await expect(page.locator('html')).toHaveAttribute('lang', 'de')
    await expect(page.locator('html')).toHaveAttribute('dir', 'ltr')
    await expect(page.getByRole('navigation', { name: 'Hauptmenü' })).toBeVisible()
    await expect.poll(() => page.title()).toBe('Einstellungen · Kora')
    await scan('settings in German')
    // A reload keeps it (per browser).
    await page.reload()
    await expect(page.getByRole('heading', { name: 'Einstellungen', level: 1 })).toBeVisible()
    // And back to English.
    await page.getByLabel('Sprache').selectOption('en')
    await expect(page.getByRole('heading', { name: 'Settings', level: 1 })).toBeVisible()
    await expect(page.locator('html')).toHaveAttribute('lang', 'en-GB')
  })

  test('the sign-in screen has the picker too, and the screen follows it', async ({ page, open }) => {
    await open('/', { scenario: 'offline', as: 'remote-passkey-admin' })
    await expect(page.getByRole('heading', { name: 'Sign in to Kora', level: 1 })).toBeVisible()
    await page.getByLabel('Language').selectOption('de')
    await expect(page.getByRole('heading', { name: 'Bei Kora anmelden', level: 1 })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Mit einem Passkey anmelden' })).toBeVisible()
  })
})

test.describe('German (beta)', () => {
  test('the main pages read in German and pass the accessibility scan', async ({ page, open, scan }) => {
    await open('/', { lang: 'de' })
    await expect(page.getByRole('heading', { name: 'Deine Server', level: 1 })).toBeVisible()
    await scan('servers list in German')
    await page.getByRole('link', { name: 'kora-live' }).first().click()
    await expect(page.getByRole('tab', { name: 'Übersicht' })).toBeVisible()
    await page.getByRole('tab', { name: 'Backups' }).click()
    await expect(page.getByRole('button', { name: 'Jetzt sichern' })).toBeVisible()
    await scan('backups tab in German')
    await open('/install', { lang: 'de' })
    await expect(page.getByLabel('Lizenzschlüssel', { exact: true })).toBeVisible()
    await scan('install wizard in German')
    await open('/system', { lang: 'de' })
    await expect(page.getByRole('heading', { name: 'Systemprüfung', level: 1 })).toBeVisible()
    await scan('system check in German')
  })

  test('numbers, dates and plurals follow the language', async ({ page, open }) => {
    await open('/servers/kora-live?tab=backups', { lang: 'de' })
    // "vor 5 Stunden" comes from Intl.RelativeTimeFormat, not from a hand-made string.
    await expect(page.getByText(/vor \d+ Stunden/).first()).toBeVisible()
  })

  test('a browser that asks for German gets German, and one that asks for something else gets English', async ({ browser }) => {
    const port = Number(process.env.E2E_MOCK_PORT ?? 4173)
    for (const [locale, heading] of [['de-AT', 'Deine Server'], ['fr-FR', 'Your servers']] as const) {
      const ctx = await browser.newContext({ locale, baseURL: `http://127.0.0.1:${port}` })
      const page = await ctx.newPage()
      await page.goto('/?scenario=default&as=local')
      await expect(page.getByRole('heading', { name: heading, level: 1 })).toBeVisible()
      await ctx.close()
    }
  })

  test('a server problem that carries a message id is shown in German; the same problem in English keeps its wording', async ({ page, open }) => {
    // The mock sends `i18n: { id: 'errors.problems.githubSignedOut' }` next to the English message (as the real panel does for the known problems).
    await open('/system', { lang: 'de' })
    await expect(page.getByText('Du bist nicht bei GitHub angemeldet.')).toBeVisible()
    await open('/system')
    await expect(page.getByText('You are not signed in to GitHub.')).toBeVisible()
  })
})

test.describe('the pseudo-language', () => {
  const pages: Array<[string, string]> = [
    ['/', 'servers'],
    ['/settings', 'settings'],
    ['/system', 'system check'],
    ['/install', 'install wizard'],
    ['/fleet', 'fleet'],
    ['/servers/kora-live', 'server overview'],
    ['/servers/kora-live?tab=backups', 'backups'],
    ['/servers/kora-live?tab=resources', 'resources'],
    ['/servers/kora-live?tab=monitoring', 'monitoring'],
    ['/servers/kora-live?tab=schedule', 'schedule'],
    ['/servers/kora-live?tab=ports', 'ports'],
    ['/servers/kora-live?tab=danger', 'danger zone'],
    ['/servers/kora-live/resources/kora_core', 'resource page'],
    ['/import', 'import'],
  ]
  for (const [path, label] of pages) {
    test(`${label}: every visible sentence is in the catalogue, and the layout holds with longer text`, async ({ page, open, scan }) => {
      await open(path, { lang: 'en-XA' })
      await expect(page.locator('h1').first()).toBeVisible()
      await page.waitForTimeout(500)
      await expect(page.locator('html')).toHaveAttribute('lang', 'en-GB-x-pseudo')
      const english = await plainEnglish(page)
      expect(english, `plain English on "${label}" (not in the catalogue):\n${english.join('\n')}`).toEqual([])
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
      expect(overflow, `${label}: longer text pushes the page ${overflow}px wider than the screen`).toBeLessThanOrEqual(1)
      await scan(`${label} in the pseudo-language`)
    })
  }

  test('a journey in the pseudo-language: the wizard to its second step, then a danger-zone dialog', async ({ page, open }) => {
    await open('/install', { lang: 'en-XA', setup: 'done' })
    // The setup is done, so a saved key is already picked.
    await page.getByRole('button', { name: pseudo('Next') }).click()
    await expect(page.getByLabel(pseudo('Server name'))).toBeVisible()
    expect(await plainEnglish(page)).toEqual([])
    await open('/servers/kora-dev?tab=danger', { lang: 'en-XA' })
    await page.getByRole('button', { name: new RegExp(`^${pseudo('Delete server')}`) }).first().click()
    await expect(page.getByRole('dialog')).toBeVisible()
    expect(await plainEnglish(page)).toEqual([])
  })
})
