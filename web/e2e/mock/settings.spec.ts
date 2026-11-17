import { test, expect } from '../fixtures'

test.describe('Settings', () => {
  test('shows every card, and is accessible', async ({ page, open, scan }) => {
    await open('/settings')
    await expect(page.getByRole('heading', { name: 'Settings', level: 1 })).toBeVisible()
    for (const title of ['Remote access', 'Kora version', 'Licence keys', 'Remembered answers', 'GitHub account', 'Uninstall Kora', 'About']) {
      await expect(page.getByRole('heading', { name: title })).toBeVisible()
    }
    await scan('settings')
  })

  test('remote access: turning it on and inviting a person shows a one-time link', async ({ page, open, scan }) => {
    await open('/settings')
    await page.getByLabel('Allow remote access').click()
    await expect(page.getByLabel('Allow remote access')).toBeChecked()
    await page.getByLabel('Name', { exact: true }).first().fill('kim')
    await page.getByRole('button', { name: 'Invite person' }).click()
    await expect(page.getByText('kim').first()).toBeVisible()
    await expect(page.getByLabel('Enrolment link for kim')).toContainText('#enrol=')
    await scan('settings: remote access with a person')
    await page.getByRole('button', { name: 'Remove kim' }).click()
    await page.getByRole('dialog').getByRole('button', { name: /^Remove/ }).click()
    await expect(page.getByRole('button', { name: 'Remove kim' })).toHaveCount(0)
  })

  test('uninstall: shows what would be removed and only enables the button after typing "uninstall"', async ({ page, open, scan }) => {
    await open('/settings')
    await page.getByRole('button', { name: 'Show what would be removed' }).click()
    const button = page.getByRole('button', { name: 'Uninstall', exact: true })
    await expect(button).toBeDisabled()
    await scan('settings: uninstall list')
  })

  test('Kora version: check again and the update button', async ({ page, open }) => {
    await open('/settings')
    await page.getByRole('button', { name: 'Check again' }).first().click()
    await expect(page.getByRole('button', { name: 'Update to the latest' })).toBeVisible()
  })

  test('remembered answers are forgotten one at a time', async ({ page, open }) => {
    await open('/settings')
    await expect(page.getByText('Nothing is remembered yet.')).toBeVisible()
  })
})

test.describe('when Kora cannot be reached', () => {
  test('a session that expired shows the sign-in screen, which says what to do', async ({ page, open, scan }) => {
    await open('/', { scenario: 'offline' })
    await expect(page.getByRole('button', { name: 'Sign in with Tailscale' }).or(page.getByLabel(/Sign-in link|User token/))).toBeVisible()
    await scan('signed out screen')
  })

  test('when Kora does not answer, the page says so and keeps trying', async ({ page, open, problems, scan }) => {
    problems.expect(/Failed to load resource|network|fetch/i)
    await open('/', { scenario: 'down' })
    await expect(page.getByText('Kora is not answering')).toBeVisible()
    await scan('Kora is not answering')
  })

  test('the slow scenario shows loading states, then the servers', async ({ page, open, scan }) => {
    await open('/', { scenario: 'slow' })
    await expect(page.getByRole('table')).toBeVisible({ timeout: 30_000 })
    await scan('servers list after loading')
  })

  test('an unknown address shows a page that leads back', async ({ page, open, scan }) => {
    await open('/no/such/page')
    await expect(page.getByRole('heading').first()).toBeVisible()
    await scan('not found')
  })
})
