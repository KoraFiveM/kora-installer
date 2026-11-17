// Shared fixtures for the mock-layer journeys.
//  - `problems`: every browser console error, page error and unhandled rejection is collected; the test fails at
//    the end if there was one (a journey that expects one removes it with `problems.expect(/text/)`).
//  - `scan(label)`: runs axe on the page as it is now and fails on serious or critical violations.
//  - `open(path, opts)`: goes to a page in a chosen mock scenario / viewer.
import { test as base, expect, type Locator, type Page } from '@playwright/test'
import AxeBuilder from '@axe-core/playwright'

export const KEY = 'cfxk_abcdefghij_12345'

export type Scenario = 'default' | 'empty' | 'fleet' | 'trouble' | 'slow' | 'offline' | 'down' | 'windows' | 'restarted'
export type Viewer = 'local' | 'operator' | 'remote-operator' | 'remote-admin' | 'remote-tailscale-admin'

class Problems {
  readonly list: string[] = []
  private allowed: RegExp[] = []
  /** This journey is expected to log something that matches `re` (an error it provokes on purpose). */
  expect(re: RegExp) {
    this.allowed.push(re)
  }
  add(text: string) {
    if (!this.allowed.some((re) => re.test(text))) this.list.push(text)
  }
}

interface Fixtures {
  problems: Problems
  open: (path: string, opts?: { scenario?: Scenario; as?: Viewer }) => Promise<void>
  scan: (label: string) => Promise<void>
}

export const test = base.extend<Fixtures>({
  problems: async ({ page }, use) => {
    const p = new Problems()
    page.on('console', (m) => {
      // The mock announces itself with console.info; only errors count.
      if (m.type() === 'error') p.add(`console.error: ${m.text()}`)
    })
    page.on('pageerror', (e) => p.add(`page error: ${e.message}`))
    // A request the browser itself cancelled (the page moved on while it was in flight) is not a failure.
    page.on('requestfailed', (r) => r.failure()?.errorText !== 'net::ERR_ABORTED' && p.add(`request failed: ${r.method()} ${r.url()} (${r.failure()?.errorText})`))
    await use(p)
    expect(p.list, 'the browser console / page must stay free of errors').toEqual([])
  },
  open: async ({ page, problems: _problems }, use) => {
    await use(async (path, opts = {}) => {
      const u = new URL(path, 'http://x')
      u.searchParams.set('scenario', opts.scenario ?? 'default')
      u.searchParams.set('as', opts.as ?? 'local')
      await page.goto(u.pathname + u.search + u.hash)
    })
  },
  scan: async ({ page }, use) => {
    await use(async (label) => {
      const r = await new AxeBuilder({ page }).exclude('#kora-mock-switcher').analyze()
      // E2E_AXE_REPORT=1 also prints the moderate and minor findings (they do not fail the test): for the accessibility backlog.
      if (process.env.E2E_AXE_REPORT) {
        for (const v of r.violations.filter((x) => x.impact !== 'serious' && x.impact !== 'critical')) console.log(`AXE[${v.impact}] ${v.id} @ ${label}: ${v.nodes.slice(0, 2).map((n) => n.target.join(' ')).join(' ; ')}`)
      }
      const bad = r.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical')
      const text = bad.map((v) => `[${v.impact}] ${v.id}: ${v.help}\n    ${v.nodes.slice(0, 4).map((n) => n.target.join(' ') + ' :: ' + (n.failureSummary ?? '').split('\n').slice(0, 3).join(' | ')).join('\n    ')}`).join('\n')
      expect(text, `accessibility violations on "${label}" (${page.url()})`).toBe('')
    })
  },
})

export { expect }

/** Short path of the wizard: key, name, then wait for the summary to settle. Returns once Install is enabled. */
export async function toSummary(page: Page, name?: string) {
  await page.getByLabel('Licence key', { exact: true }).fill(KEY)
  await page.getByRole('button', { name: 'Next' }).click()
  const box = page.getByLabel('Server name')
  if (name) await box.fill(name)
  await page.getByRole('button', { name: 'Next' }).click()
  await expect(page.getByRole('heading', { name: 'Check and install' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Install the server' })).toBeEnabled()
}

/** Settings > Licence keys: the "save a new key" form (Settings has other fields called "Name"). */
export function keyForm(page: Page) {
  return page.locator('form').filter({ has: page.getByRole('button', { name: 'Save key' }) })
}

export async function saveKey(page: Page, name: string, key: string) {
  const form = keyForm(page)
  await form.getByLabel('Name', { exact: true }).fill(name)
  await form.getByLabel('Licence key', { exact: true }).fill(key)
  await form.getByRole('button', { name: 'Save key' }).click()
}

/**
 * Keyboard only: press Tab until `target` has focus (at most `max` presses, going forward). Fails with the
 * element that had focus last, so an unreachable control shows up as a clear message.
 */
export async function tabTo(page: Page, target: Locator, max = 60) {
  for (let i = 0; i < max; i++) {
    if (await target.evaluate((el) => el === document.activeElement).catch(() => false)) return
    await page.keyboard.press('Tab')
  }
  const where = await page.evaluate(() => {
    const a = document.activeElement
    return a ? `${a.tagName.toLowerCase()} "${(a.getAttribute('aria-label') ?? a.textContent ?? '').trim().slice(0, 50)}"` : 'nothing'
  })
  throw new Error(`could not reach ${target} with the Tab key in ${max} presses (focus ended on ${where})`)
}
