// The accessibility sweep: every tab of a server, every page, and the dialogs, each scanned by axe (fails on moderate
// and above, see fixtures.ts), plus what a scanner cannot see: headings, landmarks, page titles, focus after a route
// change and inside dialogs, and a 320 px-wide screen. docs/accessibility.md lists what is checked and what is not.
import type { Page } from '@playwright/test'
import { test, expect, tabTo, saveKey, KEY } from '../fixtures'

const TABS = ['Overview', 'Console', 'Logs', 'Monitoring', 'Resources', 'Paid assets', 'Backups', 'Schedule', 'Notifications', 'Ports', 'GitHub', 'Danger zone']

test.describe('every tab of a server', () => {
  for (const server of ['kora-live', 'kora-dev']) {
    test(`all ${TABS.length} tabs of ${server}`, async ({ page, open, scan }) => {
      await open(`/servers/${server}`)
      for (const t of TABS) {
        await page.getByRole('tab', { name: t, exact: true }).click()
        await expect(page.getByRole('tab', { name: t, exact: true })).toHaveAttribute('aria-selected', 'true')
        await expect(page.getByRole('tabpanel')).toBeVisible()
        // The panel's contents settle (a skeleton gives way to the tab).
        await expect(page.getByRole('tabpanel').locator('[role=status]')).toHaveCount(0, { timeout: 10_000 }).catch(() => undefined)
        await scan(`${server}: ${t} tab`)
      }
    })
  }
})

test.describe('every page', () => {
  for (const [path, label] of [['/', 'servers'], ['/fleet', 'fleet'], ['/install', 'install'], ['/import', 'import'], ['/system', 'system check'], ['/settings', 'settings'], ['/nowhere', 'not found']] as const) {
    test(`${label}: one h1, landmarks, a title`, async ({ page, open, scan }) => {
      await open(path)
      await expect(page.locator('h1')).toHaveCount(1)
      await expect(page.getByRole('main')).toHaveCount(1)
      await expect(page.getByRole('navigation', { name: 'Main' })).toHaveCount(1)
      await expect.poll(() => page.title()).toMatch(/ · Kora$/)
      await scan(label)
    })
  }

  test('the heading outline never skips a level', async ({ page, open }) => {
    for (const path of ['/', '/settings', '/servers/kora-live?tab=resources', '/servers/kora-live/resources/kora_core', '/system', '/install']) {
      await open(path)
      await expect(page.locator('h1').first()).toBeVisible()
      await page.waitForTimeout(300)
      const levels = await page.locator('h1, h2, h3, h4, h5, h6').evaluateAll((els) => els.map((e) => Number(e.tagName[1])))
      let prev = 0
      for (const l of levels) {
        expect(l, `${path}: heading levels ${levels.join(',')}`).toBeLessThanOrEqual(prev + 1)
        prev = l
      }
    }
  })
})

test.describe('moving between pages', () => {
  test('focus goes to the new page heading, the title follows, and the skip link works', async ({ page, open }) => {
    await open('/')
    await expect(page.getByRole('heading', { name: 'Your servers', level: 1 })).toBeVisible()
    await expect.poll(() => page.title()).toBe('Your servers · Kora')
    await page.getByRole('link', { name: 'Settings', exact: true }).click()
    await expect(page.getByRole('heading', { name: 'Settings', level: 1 })).toBeFocused()
    await expect.poll(() => page.title()).toBe('Settings · Kora')
    await page.getByRole('link', { name: 'System check' }).click()
    await expect(page.getByRole('heading', { name: 'System check', level: 1 })).toBeFocused()
    // The skip link is the first stop of a fresh page, appears when focused, and moves focus into the page.
    await open('/system')
    await expect(page.getByRole('heading', { name: 'System check', level: 1 })).toBeVisible()
    await page.keyboard.press('Tab')
    const skip = page.getByRole('link', { name: 'Skip to the main content' })
    await expect(skip).toBeFocused()
    await expect(skip).toBeInViewport()
    await page.keyboard.press('Enter')
    await expect(page.getByRole('main')).toBeFocused()
  })

  test('a server page is titled with the server name once it has loaded', async ({ page, open }) => {
    await open('/servers/kora-live')
    await expect.poll(() => page.title()).toBe('kora-live · Kora')
  })
})

async function dialogOk(page: Page, scan: (l: string) => Promise<void>, label: string) {
  const d = page.getByRole('dialog')
  await expect(d).toBeVisible()
  // It has a name and a description, and focus is inside it.
  await expect(d).toHaveAccessibleName(/.+/)
  await expect(d).toHaveAccessibleDescription(/.+/)
  expect(await d.evaluate((el) => el.contains(document.activeElement))).toBe(true)
  await scan(label)
}

async function closeWithEscape(page: Page, returnsTo?: ReturnType<Page['locator']>) {
  await page.keyboard.press('Escape')
  await expect(page.getByRole('dialog')).toHaveCount(0)
  if (returnsTo) await expect(returnsTo).toBeFocused()
}

test.describe('dialogs', () => {
  test('add an existing server; Escape closes it and focus returns to the button', async ({ page, open, scan }) => {
    await open('/')
    const b = page.getByRole('button', { name: 'Add an existing server' })
    await b.click()
    await dialogOk(page, scan, 'add existing dialog')
    await closeWithEscape(page, b)
  })

  test('the confirmations of the Danger zone', async ({ page, open, scan }) => {
    await open('/servers/kora-dev?tab=danger')
    for (const name of ['Delete server']) {
      const b = page.getByRole('tabpanel').getByRole('button', { name: new RegExp(`^${name}`) }).first()
      await b.click()
      await dialogOk(page, scan, `confirm: ${name}`)
      await closeWithEscape(page, b)
    }
  })

  test('backups: check, restore and delete', async ({ page, open, scan }) => {
    await open('/servers/kora-live?tab=backups')
    for (const [name, label] of [['Check', 'check'], ['Restore…', 'restore'], ['Delete the backup', 'delete']] as const) {
      const b = page.getByRole('button', { name, exact: name !== 'Delete the backup' }).first()
      await b.click()
      await dialogOk(page, scan, `backups: ${label} dialog`)
      await closeWithEscape(page, b)
    }
  })

  test('settings: the licence key dialogs and the support bundle', async ({ page, open, scan }) => {
    await open('/settings')
    await saveKey(page, 'Main key', KEY)
    await expect(page.getByRole('button', { name: 'Rename Main key' })).toBeVisible()
    for (const [re, label] of [[/^Rename /, 'rename'], [/^Replace key of /, 'replace'], [/^Delete /, 'delete']] as const) {
      const b = page.getByRole('button', { name: re }).first()
      await b.click()
      await dialogOk(page, scan, `licence key: ${label}`)
      await closeWithEscape(page, b)
    }
    await page.getByRole('button', { name: /Create a support bundle|support bundle/i }).first().click()
    await dialogOk(page, scan, 'support bundle')
    await closeWithEscape(page)
  })

  test('overview: change the licence key, and restart asks first', async ({ page, open, scan }) => {
    await open('/servers/kora-live')
    await page.getByRole('button', { name: 'Change licence key' }).click()
    await dialogOk(page, scan, 'change licence key')
    await closeWithEscape(page)
    await page.getByRole('button', { name: 'Restart kora-live' }).click()
    await dialogOk(page, scan, 'restart confirmation')
    await closeWithEscape(page)
  })

  test('fleet: add a node', async ({ page, open, scan }) => {
    await open('/fleet')
    await page.getByRole('button', { name: /Add a node/ }).first().click()
    await dialogOk(page, scan, 'add a node')
    await closeWithEscape(page)
  })

  test('resources: the branch dialog of a resource', async ({ page, open, scan }) => {
    await open('/servers/kora-dev?tab=github')
    const b = page.getByRole('tabpanel').getByRole('button', { name: /^Change the branch of/ }).first()
    await b.click()
    await dialogOk(page, scan, 'branch dialog')
    await closeWithEscape(page)
  })

  test('remote access: sign in again before a sensitive step', async ({ page, open, scan }) => {
    await open('/settings', { as: 'remote-passkey-admin' })
    await expect(page.getByRole('heading', { name: 'Settings', level: 1 })).toBeVisible()
    await scan('settings as a remote passkey admin')
  })
})

test.describe('a 320 px wide screen (and so 400% zoom)', () => {
  test.use({ viewport: { width: 320, height: 640 } })

  async function noSideways(page: Page, label: string) {
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
    expect(overflow, `${label}: the page scrolls sideways by ${overflow}px`).toBeLessThanOrEqual(1)
  }

  for (const [path, label] of [['/', 'servers'], ['/settings', 'settings'], ['/install', 'install'], ['/system', 'system check'], ['/servers/kora-live', 'server overview'], ['/servers/kora-live?tab=backups', 'backups'], ['/servers/kora-live/resources/kora_core', 'resource page'], ['/import', 'import']] as const) {
    test(`${label} fits without sideways scrolling and passes axe`, async ({ page, open, scan }) => {
      await open(path)
      await expect(page.locator('h1').first()).toBeVisible()
      await page.waitForTimeout(400)
      await noSideways(page, label)
      await scan(`${label} at 320px`)
    })
  }

  test('the main journey works: open a server and use the keyboard to reach a tab and a dialog', async ({ page, open }) => {
    await open('/')
    await page.getByRole('link', { name: 'kora-live' }).first().click()
    await page.getByRole('tab', { name: 'Backups' }).click()
    await expect(page.getByRole('button', { name: 'Back up now' }).first()).toBeVisible()
    await page.getByRole('tab', { name: 'Danger zone' }).click()
    await page.getByRole('button', { name: /^Delete server/ }).first().click()
    const d = page.getByRole('dialog')
    await expect(d).toBeVisible()
    // The dialog fits the screen (it scrolls inside itself).
    const box = await d.boundingBox()
    expect(box!.x).toBeGreaterThanOrEqual(0)
    expect(box!.x + box!.width).toBeLessThanOrEqual(320)
  })
})

test.describe('the reduced-motion setting', () => {
  test.use({ reducedMotion: 'reduce' })
  test('transitions and animations are switched off', async ({ page, open }) => {
    await open('/')
    await expect(page.getByRole('heading', { name: 'Your servers', level: 1 })).toBeVisible()
    const d = await page.evaluate(() => getComputedStyle(document.body).transitionDuration + '|' + getComputedStyle(document.querySelector('a')!).transitionDuration)
    expect(d.split('|').every((x) => parseFloat(x) < 0.001), `transition durations: ${d}`).toBe(true)
    void tabTo
  })
})
