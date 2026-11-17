// Backups of a server in a real browser, against the mock backend: make one (with a password), check it,
// restore it (preview first, then the result, then undo), the schedule, a failing database, and what a
// remote operator may do.
import { test, expect } from '../fixtures'

const TAB = '/servers/kora-live?tab=backups'

test.describe('Backups', () => {
  test('the servers list and the Overview say when the last backup was', async ({ page, open, scan }) => {
    await open('/')
    await expect(page.getByRole('columnheader', { name: 'Last backup' })).toBeVisible()
    await expect(page.getByRole('link', { name: /Last backup 5 hours ago/ }).first()).toBeVisible()
    await scan('servers list with the last backup column')
    await open('/servers/kora-live')
    await expect(page.locator('dt', { hasText: 'Backups' })).toBeVisible()
    await page.getByRole('link', { name: /Last backup/ }).click()
    await expect(page).toHaveURL(/tab=backups/)
  })

  test('a server with no backup says so plainly and says what a backup holds', async ({ page, open, scan }) => {
    await open('/servers/adopted-existing-server', { scenario: 'fleet' })
    await page.getByRole('tab', { name: 'Backups' }).click()
    await expect(page.getByText(/A backup holds your secrets/)).toBeVisible()
    await expect(page.getByText(/^Last backup|No backup yet/).first()).toBeVisible()
    await scan('backups tab')
  })

  test('make a password-protected backup: refuses a weak password, shows progress, then the result and the warning', async ({ page, open, scan }) => {
    await open(TAB)
    await page.getByLabel('Protect the file with a password').check()
    await page.getByLabel('Password', { exact: true }).fill('short')
    await expect(page.getByText('Use at least 8 characters.')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Back up now' })).toBeDisabled()
    await scan('the password fields')
    await page.getByLabel('Password', { exact: true }).fill('a long enough password')
    await page.getByLabel('Password again').fill('a long enough password')
    await page.getByRole('button', { name: 'Back up now' }).click()
    await expect(page.getByRole('progressbar', { name: 'Backup progress' })).toBeVisible()
    await expect(page.getByText('Backup finished').first()).toBeVisible()
    await expect(page.getByText(/Keep the password safe: it cannot be recovered/)).toBeVisible()
    await expect(page.getByText('Password protected').first()).toBeVisible()
  })

  test('check a backup', async ({ page, open }) => {
    await open(TAB)
    await page.getByRole('button', { name: 'Check' }).first().click()
    const dialog = page.getByRole('dialog')
    // the newest backup is the password-protected one
    await dialog.getByLabel("This backup's password").fill('correct horse battery')
    await dialog.getByRole('button', { name: 'Check it' }).click()
    await expect(dialog.getByText('This backup is good')).toBeVisible()
    await expect(dialog.getByText(/All 212 files match their checksums/)).toBeVisible()
  })

  test('restore: preview first, a wrong password is explained, then the result, then undo', async ({ page, open, scan }) => {
    await open(TAB)
    // the newest backup is the password-protected one
    await page.getByRole('button', { name: 'Restore…' }).first().click()
    const dialog = page.getByRole('dialog')
    await dialog.getByLabel("This backup's password").fill('not the password')
    await dialog.getByRole('button', { name: 'Open the backup' }).click()
    await expect(dialog.getByText(/does not open this backup/)).toBeVisible()
    await dialog.getByLabel("This backup's password").fill('correct horse battery')
    await dialog.getByRole('button', { name: 'Open the backup' }).click()
    await expect(dialog.getByText('This will:')).toBeVisible()
    await expect(dialog.getByText(/secrets.cfg and database.json stay as they are now/)).toBeVisible()
    await expect(dialog.getByText(/safety copy of the server as it is now/)).toBeVisible()
    await scan('the restore preview')
    await dialog.getByRole('button', { name: 'Restore now' }).click()
    await expect(dialog.getByText('Restore finished')).toBeVisible()
    await expect(dialog.getByText('kora_core is back at version 4f2a9c1e')).toBeVisible()
    await dialog.locator('button', { hasText: 'Close' }).click()
    // the safety copy is there to go back to
    await page.getByRole('button', { name: /Undo this restore/ }).click()
    await page.getByLabel("This backup's password").fill('correct horse battery')
    await page.getByRole('button', { name: 'Put the server back as it was' }).click()
    await expect(page.getByText('Restore finished')).toBeVisible()
  })

  test('the schedule can be set, and says it needs the background service', async ({ page, open }) => {
    await open(TAB)
    await page.getByLabel('Once a week').check()
    await page.getByLabel('Time (24 hours)').fill('04:15')
    await expect(page.getByText(/Scheduled backups run while Kora's background service is running/)).toBeVisible()
    await page.getByRole('button', { name: 'Save' }).click()
    await expect(page.getByText('Saved.')).toBeVisible()
  })

  test('a database that is not running fails the backup with a fix button', async ({ page, open, problems }) => {
    problems.expect(/422|backup/)
    await open(TAB, { scenario: 'trouble' })
    await page.getByRole('button', { name: 'Back up now' }).click()
    await expect(page.getByText(/the database server is not running/).first()).toBeVisible()
    await expect(page.getByRole('button', { name: /Start (it|the database)/ }).first()).toBeVisible()
  })

  test('a remote operator can back up but not restore, delete or change where backups go', async ({ page, open }) => {
    await open(TAB, { as: 'remote-operator' })
    await expect(page.getByRole('button', { name: 'Back up now' })).toBeEnabled()
    await expect(page.getByRole('button', { name: 'Restore…' }).first()).toBeDisabled()
    await expect(page.getByRole('button', { name: 'Save' })).toBeDisabled()
    await expect(page.getByText(/cannot do this/).first()).toBeVisible()
  })
})
