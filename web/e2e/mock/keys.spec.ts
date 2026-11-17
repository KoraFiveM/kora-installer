import { test, expect, keyForm, saveKey, toSummary } from '../fixtures'

const MAIN = 'cfxk_MAINKEYMAINKEY12345_1111'
const TEST = 'cfxk_TESTKEYTESTKEY12345_2222'

test.describe('saved licence keys', () => {
  test('add on Settings, rename, replace and delete; only the last four characters are ever shown', async ({ page, open, scan }) => {
    await open('/settings')
    await expect(page.getByText(/No keys saved yet/)).toBeVisible()
    await expect(keyForm(page).getByRole('button', { name: 'Save key' })).toBeDisabled()
    await saveKey(page, 'Main', MAIN)
    await expect(page.getByText('Saved the key ending 1111 as "Main".')).toBeVisible()
    const list = page.getByRole('list', { name: 'Saved licence keys' })
    await expect(list.getByText(/key ending 1111/)).toBeVisible()
    await scan('settings: licence keys')

    await page.getByRole('button', { name: 'Rename Main' }).click()
    const rename = page.getByRole('dialog')
    await rename.getByLabel('New name').fill('Live')
    await rename.getByRole('button', { name: 'Rename' }).click()
    await expect(page.getByText('Renamed to "Live".')).toBeVisible()

    await page.getByRole('button', { name: 'Replace key of Live' }).click()
    const replace = page.getByRole('dialog')
    await replace.getByLabel('New licence key').fill(TEST)
    await replace.getByRole('button', { name: 'Replace key' }).click()
    await expect(page.getByText(/now holds the key ending 2222/)).toBeVisible()

    await page.getByRole('button', { name: 'Delete Live' }).click()
    const del = page.getByRole('dialog')
    await expect(del.getByText(/keep working: they have their own copy/)).toBeVisible()
    await del.getByRole('button', { name: 'Delete key' }).click()
    await expect(page.getByText(/No keys saved yet/)).toBeVisible()
    expect(await page.locator('body').innerText()).not.toMatch(/MAINKEYMAINKEY|TESTKEYTESTKEY/)
  })

  test('a name or a key that is refused is explained in plain words', async ({ page, open }) => {
    await open('/settings')
    await saveKey(page, 'Main', MAIN)
    await expect(page.getByText(/key ending 1111/).first()).toBeVisible()
    await saveKey(page, 'main', TEST)
    await expect(page.getByText('A saved key is already called "Main". Choose another name.')).toBeVisible()
    await saveKey(page, 'Other', 'not a key')
    await expect(page.getByText(/does not look like a Cfx.re licence key/)).toBeVisible()
  })

  test('the wizard offers a saved key, and the install uses it without typing it again', async ({ page, open, scan }) => {
    await open('/settings')
    await saveKey(page, 'Main', MAIN)
    await expect(page.getByText(/key ending 1111/).first()).toBeVisible()
    await page.getByRole('link', { name: 'Install new server' }).click()
    await expect(page.getByRole('heading', { name: 'Your licence key' })).toBeVisible()
    await expect(page.getByRole('radio', { name: /Main/ })).toBeChecked()
    await scan('wizard: saved licence key picked')
    await page.getByRole('button', { name: 'Next' }).click()
    await page.getByRole('button', { name: 'Next' }).click()
    await expect(page.getByRole('heading', { name: 'Check and install' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Install the server' })).toBeEnabled()
    await expect(page.getByText(/Main/).first()).toBeVisible()
  })

  test('change the key of a server: pick a saved one on a stopped server, no restart needed', async ({ page, open, scan }) => {
    await open('/settings')
    await saveKey(page, 'Main', MAIN)
    await expect(page.getByText(/key ending 1111/).first()).toBeVisible()
    await page.getByRole('link', { name: 'Servers' }).click()
    await page.getByRole('table').getByRole('link', { name: 'kora-dev' }).click()
    await page.getByRole('button', { name: 'Change licence key' }).click()
    const dlg = page.getByRole('dialog')
    await expect(dlg.getByRole('radio', { name: /Main/ })).toBeChecked()
    await expect(dlg.getByText(/stopped, so no restart is needed/)).toBeVisible()
    await scan('server overview: change licence key dialog')
    await dlg.getByRole('button', { name: 'Change licence key' }).click()
    await expect(page.getByText(/Licence key changed \(now ending 1111\)/)).toBeVisible()
    await expect(page.getByRole('button', { name: 'Restart the server now' })).toHaveCount(0)
  })

  test('a running server needs a restart after the key changes, and offers it', async ({ page, open }) => {
    await open('/settings')
    await saveKey(page, 'Main', MAIN)
    await expect(page.getByText(/key ending 1111/).first()).toBeVisible()
    await page.getByRole('link', { name: 'Servers' }).click()
    await page.getByRole('table').getByRole('link', { name: 'kora-live', exact: true }).click()
    await page.getByRole('button', { name: 'Change licence key' }).click()
    const dlg = page.getByRole('dialog')
    await expect(dlg.getByText(/only reads its key when it starts/)).toBeVisible()
    await dlg.getByRole('button', { name: 'Change licence key' }).click()
    await expect(page.getByRole('button', { name: 'Restart the server now' })).toBeVisible()
  })

  test('a new key can be pasted and saved on the way', async ({ page, open }) => {
    await open('/servers/kora-dev')
    await page.getByRole('button', { name: 'Change licence key' }).click()
    const dlg = page.getByRole('dialog')
    await dlg.getByLabel('New licence key').fill(TEST)
    await dlg.getByLabel('Name for this key').fill('Test')
    await dlg.getByRole('button', { name: 'Change licence key' }).click()
    await expect(page.getByText(/now ending 2222/)).toBeVisible()
    expect(await page.locator('body').innerText()).not.toContain('TESTKEYTESTKEY')
  })
})

