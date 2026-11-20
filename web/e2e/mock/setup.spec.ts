import { test, expect, KEY, saveKey } from '../fixtures'

// The install wizard waits for a saved Cfx.re licence key and a GitHub sign-in ("Get set up"). Only the wizard: every other page stays open.
test.describe('Get set up (the install wizard)', () => {
  test('the checklist comes first; both are finished on it, then the wizard opens', async ({ page, open, scan }) => {
    await open('/install', { setup: 'pending' })
    await expect(page.getByRole('heading', { name: 'Get set up' })).toBeVisible()
    await expect(page.getByText('Still to do')).toHaveCount(2)
    await scan('get set up: nothing done yet')

    await saveKey(page, 'Main', KEY)
    await expect(page.getByText('Still to do')).toHaveCount(1)
    await expect(page.getByText('Done', { exact: true })).toBeVisible()

    await page.getByRole('button', { name: 'Sign in to GitHub' }).click()
    await expect(page.getByText('Finish signing in on GitHub')).toBeVisible()
    // Both done: the wizard opens by itself, with the saved key picked.
    await expect(page.getByRole('heading', { name: 'Your licence key' })).toBeVisible({ timeout: 30_000 })
    await expect(page.getByRole('heading', { name: 'Get set up' })).toHaveCount(0)
    await expect(page.getByRole('radio', { name: /Main/ })).toBeChecked()
  })

  test('only the wizard waits: the servers, Settings and adding a server stay open', async ({ page, open }) => {
    await open('/', { setup: 'pending' })
    await expect(page.getByRole('link', { name: 'kora-live' }).first()).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Get set up' })).toHaveCount(0)
    await page.getByRole('link', { name: 'Settings' }).click()
    await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Get set up' })).toHaveCount(0)
  })

  test('with both done the wizard opens at once', async ({ page, open }) => {
    await open('/install')
    await expect(page.getByRole('heading', { name: 'Your licence key' })).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Get set up' })).toHaveCount(0)
  })
})
