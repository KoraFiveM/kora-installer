// The same journeys as in wizard.spec.ts, with the keyboard only: no mouse click, no `focus()` shortcut.
// Tab to a control, then Enter or Space. If a step cannot be reached this way, the test says where focus got stuck.
import { test, expect, KEY, tabTo } from '../fixtures'

test.describe('keyboard only', () => {
  test('the whole install wizard, from the licence key to "All done"', async ({ page, open, scan }) => {
    await open('/install')
    const key = page.getByLabel('Licence key', { exact: true })
    await expect(key).toBeVisible()
    await tabTo(page, key)
    await page.keyboard.type(KEY)

    const next = page.getByRole('button', { name: 'Next' })
    await tabTo(page, next)
    await page.keyboard.press('Enter')

    const name = page.getByLabel('Server name')
    await expect(name).toBeVisible()
    await tabTo(page, name)
    await page.keyboard.press('Control+A')
    await page.keyboard.type('keyboard-town')
    await tabTo(page, next)
    await page.keyboard.press('Enter')

    await expect(page.getByRole('heading', { name: 'Check and install' })).toBeVisible()
    await scan('wizard summary, reached with the keyboard')
    const install = page.getByRole('button', { name: 'Install the server' })
    await expect(install).toBeEnabled()
    await tabTo(page, install)
    await page.keyboard.press('Enter')
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })
  })

  test('a radio group is a single stop and moves with the arrow keys; Space ticks a box', async ({ page, open }) => {
    await open('/install')
    await page.getByRole('button', { name: 'Choose everything myself' }).click()
    await expect(page.getByRole('heading', { name: 'Kind of server' })).toBeVisible()
    const production = page.getByRole('radio', { name: /Production/ })
    await tabTo(page, production)
    await expect(production).toBeChecked()
    await page.keyboard.press('ArrowDown')
    await expect(page.getByRole('radio', { name: /Development/ })).toBeChecked()
    await page.keyboard.press('ArrowUp')
    await expect(production).toBeChecked()
    const advanced = page.getByRole('checkbox', { name: 'Show advanced options' })
    await tabTo(page, advanced)
    await page.keyboard.press('Space')
    await expect(advanced).toBeChecked()
  })

  test('resolving a failed step: reach "Try this step again" and carry on', async ({ page, open, scan }) => {
    await open('/install')
    await page.getByLabel('Licence key', { exact: true }).fill(KEY)
    await page.getByRole('button', { name: 'Next' }).click()
    await page.getByLabel('Server name').fill('fail-keys')
    await page.getByRole('button', { name: 'Next' }).click()
    await expect(page.getByRole('button', { name: 'Install the server' })).toBeEnabled()
    await tabTo(page, page.getByRole('button', { name: 'Install the server' }))
    await page.keyboard.press('Enter')

    await expect(page.getByRole('heading', { name: 'The install stopped' })).toBeVisible({ timeout: 40_000 })
    await scan('failed step, keyboard')
    const retry = page.getByRole('alert').getByRole('button', { name: 'Try this step again' })
    await expect(retry).toBeVisible()
    await tabTo(page, retry)
    await page.keyboard.press('Enter')
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })
  })

  test('a failed step that can be skipped: Skip reached and used with the keyboard', async ({ page, open }) => {
    await open('/install')
    await page.getByLabel('Licence key', { exact: true }).fill(KEY)
    await page.getByRole('button', { name: 'Next' }).click()
    await page.getByLabel('Server name').fill('skip-keys')
    await page.getByRole('button', { name: 'Next' }).click()
    await expect(page.getByRole('button', { name: 'Install the server' })).toBeEnabled()
    await page.getByRole('button', { name: 'Install the server' }).click()
    await expect(page.getByRole('heading', { name: 'The install stopped' })).toBeVisible({ timeout: 40_000 })
    const skip = page.getByRole('alert').getByRole('button', { name: 'Skip this step' })
    await tabTo(page, skip)
    await page.keyboard.press('Enter')
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })
  })

  test('a dialog traps focus, Escape closes it, and focus returns to the button that opened it', async ({ page, open }) => {
    await open('/servers/kora-live')
    const restart = page.getByRole('button', { name: 'Restart kora-live' })
    await tabTo(page, restart)
    await page.keyboard.press('Enter')
    const dialog = page.getByRole('dialog', { name: 'Restart kora-live?' })
    await expect(dialog).toBeVisible()
    for (let i = 0; i < 6; i++) {
      await page.keyboard.press('Tab')
      expect(await dialog.evaluate((d) => d.contains(document.activeElement))).toBe(true)
    }
    await page.keyboard.press('Escape')
    await expect(dialog).toBeHidden()
    await expect(restart).toBeFocused()
  })

  test('the server tabs follow the arrow keys', async ({ page, open }) => {
    await open('/servers/kora-live')
    await tabTo(page, page.getByRole('tab', { name: 'Overview', exact: true }))
    await page.keyboard.press('ArrowRight')
    await expect(page.getByRole('tab', { name: 'Console', exact: true })).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(page.getByRole('tab', { name: 'Console', exact: true })).toHaveAttribute('aria-selected', 'true')
  })
})
