// More keyboard-only journeys (the first ones are in keyboard.spec.ts): the resource page (commit, try a pull request),
// backups (make one, restore with its confirmation), Settings (invite a person) and signing in with a one-time code.
// Rule: no mouse click and no `fill()`. Reach a control with Tab, use Enter or Space, type with the keyboard.
import type { Locator, Page } from '@playwright/test'
import { test, expect, tabTo } from '../fixtures'

// About signed-out and key-less states, not the setup checklist: start with the setup undone.
test.use({ setupState: 'pending' })

/** Tab to a field, select what is in it and type over it. */
async function typeInto(page: Page, field: Locator, text: string) {
  await tabTo(page, field)
  await page.keyboard.press('Control+A')
  await page.keyboard.type(text)
}
async function press(page: Page, control: Locator, key = 'Enter', max = 200) {
  await tabTo(page, control, max)
  await page.keyboard.press(key)
}

/** Like press(), going backwards (Shift+Tab): for a control above the place focus was left in. */
async function pressBack(page: Page, control: Locator, key = 'Enter') {
  for (let i = 0; i < 120; i++) {
    if (await control.evaluate((el) => el === document.activeElement).catch(() => false)) return page.keyboard.press(key)
    await page.keyboard.press('Shift+Tab')
  }
  throw new Error(`could not reach ${control} with Shift+Tab`)
}

test.describe('keyboard only: the resource page', () => {
  test('sign in to GitHub, save changes with a message, and open the pull request list', async ({ page, open, scan }) => {
    await open('/servers/kora-dev?tab=github')
    await press(page, page.getByRole('tabpanel').getByRole('button', { name: 'Sign in to GitHub' }))
    await expect(page.getByRole('tabpanel').getByText('Signed in', { exact: true })).toBeVisible({ timeout: 30_000 })
    // To the resource's page by its link.
    await open('/servers/kora-dev/resources/kora_jobs')
    await expect(page.getByRole('heading', { name: 'kora_jobs', level: 1 })).toBeVisible()
    // The changed files are a labelled list, and the diff is reachable by its button.
    await expect(page.getByRole('list', { name: 'Changed files' })).toBeVisible()
    await press(page, page.getByRole('button', { name: 'Show the changes line by line' }))
    await expect(page.getByLabel('Changes, line by line')).toContainText('+Config.Pay = 120')
    await scan('resource page, keyboard: the diff')
    await typeInto(page, page.getByLabel(/Save these changes with a message/), 'Raise the pay')
    await press(page, page.getByRole('button', { name: /Save the changes/ }))
    await expect(page.getByText(/Saved 3 files as commit/)).toBeVisible()
  })

  test('try a pull request: the tick is a Space away, then the button works', async ({ page, open }) => {
    await open('/servers/kora-live?tab=github')
    await press(page, page.getByRole('tabpanel').getByRole('button', { name: 'Sign in to GitHub' }))
    await expect(page.getByRole('tabpanel').getByText('Signed in', { exact: true })).toBeVisible({ timeout: 30_000 })
    // The tabs are one stop (the selected one); the arrow keys move along them.
    await pressBack(page, page.getByRole('tab', { name: 'GitHub', exact: true }), 'Home')
    await expect(page.getByRole('tab', { name: 'Overview', exact: true })).toBeFocused()
    for (const next of ['Console', 'Logs', 'Monitoring', 'Resources']) {
      await page.keyboard.press('ArrowRight')
      await expect(page.getByRole('tab', { name: next, exact: true })).toBeFocused()
    }
    await expect(page.getByRole('tab', { name: 'Resources', exact: true })).toBeFocused()
    await page.keyboard.press('Enter')
    await press(page, page.getByRole('link', { name: 'Open the page of kora_core' }))
    await expect(page.getByRole('heading', { name: 'kora_core', level: 1 })).toBeFocused()
    const panel = page.getByRole('heading', { name: 'Pull requests' }).locator('xpath=ancestor::div[contains(@class,"rounded")][1]')
    const tryIt = panel.getByRole('button', { name: 'Try it: pull request #41' })
    await expect(tryIt).toBeDisabled()
    await press(page, panel.getByLabel(/I understand: this resource will run unreleased code/), 'Space')
    await expect(tryIt).toBeEnabled()
    await press(page, tryIt)
    await expect(page.getByText(/Now on pull request #41/)).toBeVisible()
  })
})

test.describe('keyboard only: backups', () => {
  test('make a backup, then restore one: the confirmation step is reachable and Escape leaves it', async ({ page, open, scan }) => {
    await open('/servers/kora-live?tab=backups')
    await press(page, page.getByRole('button', { name: 'Back up now' }))
    await expect(page.getByRole('progressbar', { name: 'Backup progress' })).toBeVisible()
    await expect(page.getByText('Backup finished').first()).toBeVisible()
    const restore = page.getByRole('button', { name: 'Restore…' }).first()
    await press(page, restore)
    const dialog = page.getByRole('dialog')
    await expect(dialog).toBeVisible()
    // The dialog's first stop is inside it, and the password field is reached with Tab.
    expect(await dialog.evaluate((d) => d.contains(document.activeElement))).toBe(true)
    const password = dialog.getByLabel("This backup's password")
    // A backup without a password is previewed at once; a protected one waits for its password.
    await expect(password.or(dialog.getByText('This will:'))).toBeVisible()
    if (await password.isVisible()) {
      await typeInto(page, password, 'correct horse battery')
      await press(page, dialog.getByRole('button', { name: 'Open the backup' }))
    }
    await expect(dialog.getByText('This will:')).toBeVisible()
    await scan('restore confirmation, keyboard')
    // "Restore now" is a deliberate second step: reachable, but nothing happens before Enter on it.
    await expect(dialog.getByRole('button', { name: 'Restore now' })).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(dialog).toHaveCount(0)
    await expect(restore).toBeFocused()
  })
})

test.describe('keyboard only: settings', () => {
  test('turn remote access on and invite a person', async ({ page, open }) => {
    await open('/settings')
    await press(page, page.getByLabel('Allow remote access'), 'Space')
    await expect(page.getByLabel('Allow remote access')).toBeChecked()
    await typeInto(page, page.getByLabel('Name', { exact: true }).first(), 'kim')
    // Enter in the field submits the form.
    await page.keyboard.press('Enter')
    await expect(page.getByLabel('Enrolment link for kim')).toContainText('#enrol=')
  })
})

test.describe('keyboard only: signing in with a one-time code', () => {
  test('from the first field to the panel, and a refusal is announced', async ({ page, open, scan }) => {
    await open('/', { scenario: 'offline', as: 'remote-passkey-admin' })
    await expect(page.getByRole('heading', { name: 'Sign in to Kora', level: 1 })).toBeVisible()
    await press(page, page.getByRole('button', { name: 'Use a one-time code instead' }))
    await typeInto(page, page.getByLabel('Your name'), 'sam')
    await typeInto(page, page.getByLabel('Password'), 'wrong')
    await typeInto(page, page.getByLabel('One-time code'), '123456')
    await page.keyboard.press('Enter')
    // The refusal is an alert (read out at once), and the fields are still there to try again.
    await expect(page.getByRole('alert').filter({ hasText: /not right/ })).toBeVisible()
    await scan('sign-in refused, keyboard')
    await typeInto(page, page.getByLabel('Password'), 'plum otter lantern quietly')
    await typeInto(page, page.getByLabel('One-time code'), '123456')
    await page.keyboard.press('Enter')
    await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible()
  })
})
