// The core journey, run in a second engine (WebKit) as well as Chromium: no servers -> wizard -> a finished install.
// Cheap on purpose: one test. The full set runs in Chromium only (see playwright.config.ts).
import { test, expect, toSummary } from '../fixtures'

test('first run to a finished install', async ({ page, open }) => {
  await open('/', { scenario: 'empty' })
  await expect(page.getByRole('heading', { name: 'Your licence key' })).toBeVisible()
  await toSummary(page, 'cross-town')
  await page.getByRole('button', { name: 'Install the server' }).click()
  await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })
  await page.getByRole('link', { name: 'Servers' }).click()
  await expect(page.getByRole('link', { name: 'cross-town' })).toBeVisible()
})
