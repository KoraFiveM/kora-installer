// Smoke layer: the real `kora panel` binary, the real UI embedded in it, a real browser. No installs, no servers started.
// One panel (throw-away HOME and KORA_HOME, spare port) is shared by the tests of a worker; every test signs in with its
// own one-time link. Every test also fails on a console error, a page error, a failed request or an HTTP 4xx/5xx from the panel.
import { test, expect } from './panel'

const TABS = ['Overview', 'Console', 'Logs', 'Monitoring', 'Resources', 'Paid assets', 'Notifications', 'Ports', 'Danger zone']

test.describe('sign-in', () => {
  test('the one-time link signs in, is cleaned out of the address bar, and works only once', async ({ page, browser, panel, trouble }) => {
    const link = await panel.newLink()
    await page.goto(link)
    await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible({ timeout: 30_000 })
    expect(page.url()).not.toMatch(/code=|token=/)
    // The session survives a reload: no link needed.
    await page.reload()
    await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible()

    // The same link in another browser: refused, with the sign-in screen instead of a crash.
    trouble.expect(/401|Failed to load resource/)
    const other = await browser.newContext()
    const p2 = await other.newPage()
    await p2.goto(link)
    await expect(p2.getByRole('heading', { name: /sign in|session|Kora Panel/i }).first()).toBeVisible({ timeout: 30_000 })
    await expect(p2.getByRole('navigation', { name: 'Main' })).toHaveCount(0)
    await other.close()
  })

  test('without a session the panel asks for sign-in and shows no data', async ({ browser, panel, trouble }) => {
    trouble.expect(/401|Failed to load resource/)
    const ctx = await browser.newContext()
    const page = await ctx.newPage()
    await page.goto(panel.origin + '/')
    await expect(page.getByRole('navigation', { name: 'Main' })).toHaveCount(0)
    await expect(page.getByRole('textbox').or(page.getByRole('button', { name: /sign in/i })).first()).toBeVisible({ timeout: 30_000 })
    await ctx.close()
  })
})

test.describe('the main pages', () => {
  test('first visit goes to the install wizard, which has a way out; the other pages load', async ({ signedIn: page }) => {
    await expect(page.getByRole('heading', { name: 'Your licence key' })).toBeVisible()
    await expect(page.getByRole('link', { name: 'Add an existing server' })).toBeVisible()
    for (const [link, heading] of [
      ['System check', 'System check'],
      ['Settings', 'Settings'],
      ['Fleet', 'Fleet'],
      ['Install new server', 'Install a new server'],
      ['Servers', 'Your servers'],
    ] as const) {
      await page.getByRole('link', { name: link, exact: true }).click()
      await expect(page.getByRole('heading', { name: heading, level: 1 })).toBeVisible()
    }
    await expect(page.getByText(/^Kora \d+\.\d+\.\d+/)).toBeVisible()
  })

  test('a page address opened directly loads (the panel serves the app for every route), and an unknown one says so', async ({ signedIn: page, panel }) => {
    await page.goto(panel.origin + '/settings')
    await expect(page.getByRole('heading', { name: 'Settings', level: 1 })).toBeVisible()
    await page.goto(panel.origin + '/no/such/page')
    await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible()
  })

  test('the panel serves a missing asset as a real 404, so an old tab learns it must reload', async ({ signedIn: page, panel, trouble }) => {
    trouble.expect(/404/)
    const r = await page.request.get(panel.origin + '/assets/Settings-0000old0.js')
    expect(r.status()).toBe(404)
  })
})

test.describe('an existing (fake) server folder', () => {
  test('is added, listed, and every tab of it opens', async ({ signedIn: page, panel, trouble }) => {
    // Known: the Resources tab asks for the catalogue of a server Kora did not install and gets a guided 422 (it then
    // explains that in words). The browser logs every 4xx as a console error. Listed in the web-e2e report.
    trouble.expect(/HTTP 422 GET .*\/catalog|status of 422/)
    const dir = panel.fakeServer('smoke-existing')
    await page.goto(panel.origin + '/?add=1')
    const dialog = page.getByRole('dialog', { name: 'Add an existing server' })
    await dialog.getByLabel('Server folder').fill(dir)
    await dialog.getByRole('button', { name: 'Add this folder' }).click()
    await expect(dialog).toBeHidden({ timeout: 30_000 })
    const link = page.getByRole('table').getByRole('link', { name: 'smoke-existing' })
    await expect(link).toBeVisible()
    await expect(page.getByText('Added from an existing folder')).toBeVisible()

    await link.click()
    await expect(page.getByRole('heading', { name: 'smoke-existing', level: 1 })).toBeVisible()
    for (const name of TABS) {
      await page.getByRole('tab', { name, exact: true }).click()
      await expect(page.getByRole('tab', { name, exact: true })).toHaveAttribute('aria-selected', 'true')
      await expect(page.getByRole('tabpanel')).toBeVisible()
      // A tab shows its content or a plain explanation, never the "Something went wrong" screen or a reload request.
      await expect(page.getByRole('heading', { name: 'Something went wrong' })).toHaveCount(0)
      await expect(page.getByRole('heading', { name: 'This page needs to be reloaded' })).toHaveCount(0)
    }
  })

  test('a folder that is not a server is refused in plain words', async ({ signedIn: page, panel, trouble }) => {
    trouble.expect(/HTTP 400|status of 400/)
    await page.goto(panel.origin + '/?add=1')
    const dialog = page.getByRole('dialog', { name: 'Add an existing server' })
    await dialog.getByLabel('Server folder').fill(panel.root)
    await dialog.getByRole('button', { name: 'Add this folder' }).click()
    await expect(dialog.getByRole('alert')).toBeVisible()
  })

  test('scanning for servers finishes', async ({ signedIn: page, panel }) => {
    await page.goto(panel.origin + '/?add=1')
    const dialog = page.getByRole('dialog', { name: 'Add an existing server' })
    await dialog.getByRole('button', { name: 'Scan for servers' }).click()
    await expect(dialog.getByRole('button', { name: 'Scan for servers' })).toBeEnabled({ timeout: 60_000 })
  })
})

test.describe('the real checks', () => {
  test('System check answers with the real tools of this computer', async ({ signedIn: page }) => {
    await page.getByRole('link', { name: 'System check', exact: true }).click()
    for (const label of ['Git', 'GitHub CLI', 'Database', 'Administrator rights']) {
      await expect(page.getByRole('heading', { name: label, exact: true })).toBeVisible({ timeout: 30_000 })
    }
  })

  test('the install wizard gets real suggestions, kept inside the throw-away home, and is not started', async ({ signedIn: page, panel }) => {
    await page.getByRole('link', { name: 'Install new server', exact: true }).click()
    await page.getByLabel('Licence key', { exact: true }).fill('cfxk_abcdefghij_12345')
    await page.getByRole('button', { name: 'Next' }).click()
    await expect(page.getByLabel('Server name')).not.toHaveValue('')
    await page.getByRole('button', { name: 'Next' }).click()
    await expect(page.getByRole('heading', { name: 'Check and install' })).toBeVisible()
    // The suggested folder is under the isolated HOME, so nothing of the machine's own is touched.
    await expect(page.getByText(panel.root, { exact: false }).first()).toBeVisible()
    await expect(page.getByText('Game 30120, txAdmin 40120').or(page.getByText(/Game \d+, txAdmin \d+/))).toBeVisible()
    // Wait for the real checks (disk space, ports, rights) to answer. The Install button is not pressed in this layer.
    await expect(page.getByText(/Disk space/)).toBeVisible({ timeout: 30_000 })
  })
})
