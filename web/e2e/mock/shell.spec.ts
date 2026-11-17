import { test, expect } from '../fixtures'

// "Inside the Kora desktop app" is simulated with ?shell=1 (src/api/mock/shell.ts): the page gets window.koraShell.
test.describe('inside the desktop app (mock)', () => {
  test('Browse fills the folder with what the system dialog chose', async ({ page, open, scan }) => {
    await open('/?shell=1')
    await page.getByRole('button', { name: 'Add an existing server' }).click()
    const dialog = page.getByRole('dialog', { name: 'Add an existing server' })
    await dialog.getByRole('button', { name: 'Browse...' }).click()
    await expect(dialog.getByLabel('Server folder')).toHaveValue('C:\\FiveM\\my-server')
    await scan('add an existing server, inside the app')
    const calls = await page.evaluate(() => (window as unknown as { __koraShellCalls?: { cmd: string }[] }).__koraShellCalls)
    expect(calls?.map((c) => c.cmd)).toContain('pick_folder')
  })

  test('a plain browser has no Browse button and no bridge', async ({ page, open }) => {
    await open('/')
    await page.getByRole('button', { name: 'Add an existing server' }).click()
    await expect(page.getByRole('dialog').getByRole('button', { name: 'Browse...' })).toHaveCount(0)
    expect(await page.evaluate(() => 'koraShell' in window)).toBe(false)
  })
})
