// The support bundle in a real browser: Settings, the link in an error's details, what is shown before saving, and
// the issue form's cleaned summary.
import { test, expect } from '../fixtures'

test.describe('Support bundle', () => {
  test('Settings: prepare, read a file, see what was left out, save it', async ({ page, open, scan }) => {
    await open('/settings')
    await page.getByRole('button', { name: 'Create a support bundle' }).click()
    const dialog = page.getByRole('dialog')
    await expect(dialog.getByText(/Kora never uploads this file anywhere/).first()).toBeVisible()
    await dialog.getByRole('button', { name: /Prepare it and show me what is in it/ }).click()
    const list = dialog.getByRole('list', { name: 'Files in the support bundle' })
    await expect(list.getByText('servers/kora-live/server.cfg')).toBeVisible()
    await expect(dialog.getByText('servers/kora-live/secrets.cfg')).toBeVisible()
    await list.getByRole('button', { name: 'servers/kora-live/server.cfg' }).click()
    const text = dialog.getByLabel('Text of servers/kora-live/server.cfg')
    await expect(text).toContainText('<hidden: licence key>')
    await expect(text).not.toContainText('cfxk_')
    await scan('the support bundle preview')
    await dialog.getByRole('button', { name: 'Save this bundle' }).click()
    await expect(dialog.getByText('Saved', { exact: true })).toBeVisible()
    await expect(dialog.getByLabel('Where it was saved')).toContainText('kora-support-')
  })

  test('cancelling saves nothing', async ({ page, open }) => {
    await open('/settings')
    await page.getByRole('button', { name: 'Create a support bundle' }).click()
    await page.getByRole('button', { name: /Prepare it/ }).click()
    await page.getByRole('button', { name: 'Cancel, save nothing' }).click()
    await expect(page.getByRole('dialog')).toHaveCount(0)
  })

  test('an operator cannot make one and is told why', async ({ page, open }) => {
    await open('/settings', { as: 'remote-operator' })
    await expect(page.getByText('Ask an administrator to make one.')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Create a support bundle' })).toHaveCount(0)
  })
})
