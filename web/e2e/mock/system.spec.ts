import { test, expect } from '../fixtures'

// About signed-out and key-less states, not the setup checklist: start with the setup undone.
test.use({ setupState: 'pending' })

const line = (page: import('@playwright/test').Page, label: string) => page.getByRole('heading', { name: label, exact: true }).locator('xpath=ancestor::li[1]')

test.describe('System check', () => {
  test('lists the tools in plain words and says what needs fixing', async ({ page, open, scan }) => {
    await open('/system')
    await expect(page.getByRole('heading', { name: 'System check' })).toBeVisible()
    for (const label of ['Git', 'GitHub CLI', 'GitHub sign-in', 'Database', 'Administrator rights', 'Background service']) {
      await expect(page.getByRole('heading', { name: label, exact: true })).toBeVisible()
    }
    await expect(line(page, 'Git')).toContainText(/Installed 2\.45\.1/)
    await expect(line(page, 'GitHub sign-in').getByRole('button', { name: 'Sign in to GitHub' })).toBeVisible()
    await expect(page.getByText('1 thing needs fixing.')).toBeVisible()
    await scan('system check')
  })

  test('installs the GitHub CLI: a failed download offers Try again, and the page updates by itself', async ({ page, open, scan }) => {
    await open('/system', { scenario: 'trouble' })
    const gh = line(page, 'GitHub CLI')
    await expect(gh).toContainText('Not installed.')
    await expect(line(page, 'GitHub sign-in').getByRole('button')).toHaveCount(0)
    await gh.getByRole('button', { name: 'Install the GitHub CLI for me' }).click()
    const dialog = page.getByRole('dialog', { name: 'Install the GitHub CLI' })
    await expect(dialog.getByText(/checked against a known checksum/)).toBeVisible()
    await expect(dialog.getByText(/Kora could not download the GitHub CLI/)).toBeVisible()
    await expect(dialog.getByRole('link', { name: /cli\.github\.com/ })).toBeVisible()
    await scan('system check: failed GitHub CLI download')
    await dialog.getByRole('button', { name: 'Try again' }).click()
    await expect(dialog.getByText('Done')).toBeVisible()
    await dialog.getByRole('button', { name: 'Close' }).first().click()
    await expect(dialog).toBeHidden()
    await expect(line(page, 'GitHub CLI')).toContainText(/Installed 2\.104\.0/)
    await expect(line(page, 'GitHub sign-in').getByRole('button', { name: 'Sign in to GitHub' })).toBeVisible()
  })

  test('shows the administrator command when a fix needs rights Kora does not have', async ({ page, open, scan }) => {
    await open('/system', { scenario: 'trouble' })
    await line(page, 'Database').getByRole('button', { name: 'Start it for me' }).click()
    const dialog = page.getByRole('dialog', { name: 'Start the database' })
    await expect(dialog.getByText(/needs administrator rights/).first()).toBeVisible()
    await expect(dialog.getByLabel('Command to run')).toContainText(/kora check --start-database/)
    await scan('system check: administrator command')
  })

  test('"Check again" asks again', async ({ page, open }) => {
    await open('/system')
    await expect(line(page, 'Database')).toBeVisible()
    await page.getByRole('button', { name: 'Check again' }).click()
    await expect(line(page, 'Database')).toBeVisible()
  })

  test('an operator is told who can fix things, without fix buttons', async ({ page, open, scan }) => {
    await open('/system', { scenario: 'trouble', as: 'operator' })
    await expect(page.getByRole('heading', { name: 'System check' })).toBeVisible()
    await expect(line(page, 'Database')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Start it for me' })).toHaveCount(0)
    await scan('system check as operator')
  })
})
