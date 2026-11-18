import { test, expect } from '../fixtures'

test.describe('first run', () => {
  test('with no servers the panel opens the install wizard, which has a way out', async ({ page, open, scan }) => {
    await open('/', { scenario: 'empty' })
    await expect(page.getByRole('heading', { name: 'Your licence key' })).toBeVisible()
    await expect(page).toHaveURL(/\/install/)
    await scan('install wizard, first step')
    await page.getByRole('link', { name: 'Add an existing server' }).click()
    const dialog = page.getByRole('dialog', { name: 'Add an existing server' })
    await expect(dialog).toBeVisible()
    await scan('add an existing server dialog')

    // Scan finds folders; adding one lands on the list.
    await dialog.getByRole('button', { name: 'Scan for servers' }).click()
    await expect(dialog.getByRole('button', { name: 'Add weekend-test' })).toBeVisible()
    await dialog.getByRole('button', { name: 'Add weekend-test' }).click()
    await expect(dialog).toBeHidden()
    await expect(page.getByRole('link', { name: 'weekend-test' })).toBeVisible()
    await expect(page.getByText('Added from an existing folder')).toBeVisible()
    await scan('servers list with one added server')
  })

  test('typing a folder that is not a server says why, in plain words', async ({ page, open }) => {
    await open('/?add=1', { scenario: 'empty' })
    const dialog = page.getByRole('dialog', { name: 'Add an existing server' })
    await dialog.getByLabel('Server folder').fill('/srv/nothing-here')
    await dialog.getByRole('button', { name: 'Add this folder' }).click()
    await expect(dialog.getByRole('alert')).toContainText('does not look like a FiveM server')
    await dialog.getByLabel('Server folder').fill('/srv/fivem/my-own')
    await dialog.getByRole('button', { name: 'Add this folder' }).click()
    await expect(page.getByRole('link', { name: 'my-own' })).toBeVisible()
  })

  test('the wizard is only forced once per tab', async ({ page, open }) => {
    await open('/', { scenario: 'empty' })
    await expect(page).toHaveURL(/\/install/)
    await page.getByRole('link', { name: 'Servers' }).click()
    await expect(page.getByRole('heading', { name: 'No servers yet' })).toBeVisible()
    await expect(page).toHaveURL(/\/$/)
  })
})
