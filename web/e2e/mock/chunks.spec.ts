// A page or tab file that cannot be loaded (Kora updated itself while this tab was open: the old page asks for files
// the new binary does not have, and the panel answers 404 for them) must offer "Reload", never a blank page.
import { test, expect } from '../fixtures'

const GONE = { status: 404, contentType: 'text/plain', body: 'not found' }
const FAILED_FILE = /Failed to load resource|dynamically imported module|404|request failed: GET .*\/assets\//i

test.describe('a page file that no longer exists', () => {
  test('Settings: the shell stays, the page offers Reload, and Reload brings the page back', async ({ page, open, problems }) => {
    problems.expect(FAILED_FILE)
    await open('/')
    await expect(page.getByRole('table')).toBeVisible()
    let gone = true
    await page.route('**/assets/SettingsPage-*.js', (route) => (gone ? route.fulfill(GONE) : route.continue()))

    await page.getByRole('link', { name: 'Settings' }).click()
    await expect(page.getByRole('heading', { name: 'This page needs to be reloaded' })).toBeVisible()
    await expect(page.getByText(/Kora was probably updated/)).toBeVisible()
    // Not blank: the menu is still there and other pages still work.
    await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible()
    await page.getByRole('link', { name: 'System check' }).click()
    await expect(page.getByRole('heading', { name: 'System check' })).toBeVisible()

    gone = false // the new version of the file is there after a reload
    await page.getByRole('link', { name: 'Settings' }).click()
    await expect(page.getByRole('heading', { name: 'This page needs to be reloaded' })).toBeVisible()
    await page.getByRole('button', { name: 'Reload the page' }).click()
    await expect(page.getByRole('heading', { name: 'Settings', level: 1 })).toBeVisible()
  })

  test('the install wizard (opened by address) offers Reload too', async ({ page, open, problems, scan }) => {
    problems.expect(FAILED_FILE)
    await page.route('**/assets/InstallWizard-*.js', (route) => route.fulfill(GONE))
    await open('/install')
    await expect(page.getByRole('heading', { name: 'This page needs to be reloaded' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Reload the page' })).toBeVisible()
    await scan('chunk load error screen')
  })

  test('a server tab that cannot load says so inside the tab, and the other tabs keep working', async ({ page, open, problems }) => {
    problems.expect(FAILED_FILE)
    await page.route('**/assets/ConsoleTab-*.js', (route) => route.fulfill(GONE))
    await open('/servers/kora-live')
    await page.getByRole('tab', { name: 'Console', exact: true }).click()
    await expect(page.getByRole('heading', { name: 'This page needs to be reloaded' })).toBeVisible()
    await page.getByRole('tab', { name: 'Ports', exact: true }).click()
    await expect(page.getByRole('tabpanel')).not.toContainText('needs to be reloaded')
    await expect(page.getByRole('heading', { name: 'kora-live', level: 1 })).toBeVisible()
  })

  test('a slow file: the page you are on stays until the next one is ready, and a first visit shows a loading state', async ({ page, open }) => {
    let release: () => void = () => undefined
    const held = new Promise<void>((r) => (release = r))
    await page.route('**/assets/SettingsPage-*.js', async (route) => {
      await held
      await route.continue()
    })
    // Opened by address: nothing to keep on screen, so the loading state shows.
    await open('/settings')
    await expect(page.getByRole('status').filter({ hasText: 'Loading…' })).toBeAttached()
    await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible()
    release()
    await expect(page.getByRole('heading', { name: 'Settings', level: 1 })).toBeVisible()
    // Moving within the app: the current page stays (never blank) until the next file arrives.
    await page.getByRole('link', { name: 'System check' }).click()
    await expect(page.getByRole('heading', { name: 'System check' })).toBeVisible()
  })
})
