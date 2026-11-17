// "Bring a server from a backup or another machine" in a real browser, against the mock backend: the entry link in
// the install wizard, uploading a file, its password, the preview (clashes, licence warning, what does not come
// along), the import, the result with the old-computer instructions, and a damaged file.
import { test, expect } from '../fixtures'

const file = (name: string) => ({ name, mimeType: 'application/octet-stream', buffer: Buffer.from('not really a backup, the mock only reads the name') })

test.describe('Bring a server in', () => {
  test('the install wizard links to it', async ({ page, open }) => {
    await open('/install')
    await page.getByRole('link', { name: 'Bring a server from a backup or another machine' }).click()
    await expect(page).toHaveURL(/\/import$/)
    await expect(page.getByRole('heading', { name: 'Bring a server from a backup or another machine' })).toBeVisible()
  })

  test('upload, password, preview, fix a clash, import, done', async ({ page, open, scan }) => {
    await open('/import')
    await expect(page.getByText(/Nothing leaves your computers except the file you carry yourself/)).toBeVisible()
    await scan('the file step')
    await page.setInputFiles('#import-file', file('old-rp.tar.gz.age'))
    await page.getByLabel('Its password').fill('wrong password here')
    await page.getByRole('button', { name: 'Open the file' }).click()
    await expect(page.getByText(/does not open this backup/)).toBeVisible()
    await page.getByLabel('Its password').fill('correct horse battery')
    await page.getByRole('button', { name: 'Open the file' }).click()
    await expect(page.getByText('Kora will create:')).toBeVisible()
    await expect(page.getByText(/Cfx.re ties a key to your account/)).toBeVisible()
    await expect(page.getByText('made on Windows')).toBeVisible()
    await expect(page.getByText(/The operating-system service that starts the server with the computer/)).toBeVisible()
    await expect(page.getByText(/different operating system/).first()).toBeVisible()
    await scan('the import preview')
    // the clash: the game port is in use, Kora suggests another
    await expect(page.getByText(/The game port 30120 is already used/)).toBeVisible()
    await page.getByRole('button', { name: 'Use 30121' }).click()
    await expect(page.getByLabel('Game port')).toHaveValue('30121')
    await page.getByLabel('Use one of my saved keys instead').check()
    await page.getByLabel('Saved licence key').selectOption('Spare')
    await page.getByRole('button', { name: 'Bring the server in' }).click()
    await expect(page.getByRole('progressbar', { name: 'Import progress' })).toBeVisible()
    await expect(page.getByText('Old RP server is imported')).toBeVisible()
    await expect(page.getByText(/Remove it from the old computer/)).toBeVisible()
    await expect(page.getByText(/stop it on the OLD computer/)).toBeVisible()
    await scan('the import result')
    await page.getByRole('link', { name: 'Open Old RP server' }).click()
    await expect(page).toHaveURL(/servers\/Old%20RP%20server/)
  })

  test('a damaged file is explained', async ({ page, open, problems }) => {
    problems.expect(/422|backup/)
    await open('/import')
    await page.setInputFiles('#import-file', file('broken.tar.gz'))
    await expect(page.getByText(/file is damaged or incomplete/)).toBeVisible()
  })

  test('an operator is told to ask an administrator', async ({ page, open }) => {
    await open('/import', { as: 'remote-operator' })
    await expect(page.getByText(/cannot bring a server in/)).toBeVisible()
    await expect(page.locator('#import-file')).toHaveCount(0)
  })

  test('moving a server out: the Backups tab asks for a password and makes the file', async ({ page, open }) => {
    await open('/servers/kora-live?tab=backups')
    await expect(page.getByText('Move this server to another computer')).toBeVisible()
    await page.getByLabel('Password for the file').fill('a long enough password')
    await page.getByLabel('Password again').last().fill('a long enough password')
    await page.getByRole('button', { name: 'Make the file to move' }).click()
    await expect(page.getByText('The file is ready')).toBeVisible()
  })
})
