import { test, expect, toSummary } from '../fixtures'

const installButton = (page: import('@playwright/test').Page) => page.getByRole('button', { name: 'Install the server' })

test.describe('install on Windows: Kora asks Windows for permission itself', () => {
  test('says up front that Windows will ask once, and what for', async ({ page, open, scan }) => {
    await open('/install', { scenario: 'windows' })
    await toSummary(page, 'my-town')
    const notice = page.getByRole('status').filter({ hasText: 'Windows will ask for permission once' })
    await expect(notice).toContainText('Register the server as a Windows service')
    await expect(notice).toContainText('Open the game port in the firewall')
    await expect(notice).toContainText('flashing shield icon')
    await scan('wizard on Windows: permission notice')
  })

  test('"Install without these parts" turns them off before anything starts', async ({ page, open }) => {
    await open('/install', { scenario: 'windows' })
    await toSummary(page, 'my-town')
    await page.getByRole('button', { name: 'Install without these parts' }).click()
    await expect(page.getByText('Windows will ask for permission once')).toHaveCount(0)
    await expect(page.getByText('Leave closed')).toBeVisible()
    await expect(installButton(page)).toBeEnabled()
  })

  test('the permission step is part of the install, which finishes', async ({ page, open }) => {
    await open('/install', { scenario: 'windows' })
    await toSummary(page, 'my-town')
    await installButton(page).click()
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })
    await expect(page.getByText('Administrator permission')).toBeVisible()
  })

  test('a No at the prompt is not a dead end: "Ask again" asks again and the install carries on', async ({ page, open, scan }) => {
    await open('/install', { scenario: 'windows' })
    await toSummary(page, 'decline-town')
    await installButton(page).click()
    await expect(page.getByRole('heading', { name: 'The install stopped' })).toBeVisible({ timeout: 40_000 })
    const alert = page.getByRole('alert').filter({ hasText: 'Windows did not get permission' })
    await expect(alert.getByRole('button', { name: 'Ask again' })).toBeVisible()
    await expect(alert.getByRole('button', { name: 'Continue without the administrator parts' })).toBeVisible()
    await expect(alert.getByText('Show me the command')).toBeVisible()
    await scan('wizard on Windows: permission declined')
    await alert.getByRole('button', { name: 'Ask again' }).click()
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })
  })

  test('nobody answered: "Continue without the administrator parts" carries on as the person, and says what is missing', async ({ page, open }) => {
    await open('/install', { scenario: 'windows' })
    await toSummary(page, 'nobody-town')
    await installButton(page).click()
    await expect(page.getByRole('heading', { name: 'The install stopped' })).toBeVisible({ timeout: 40_000 })
    const alert = page.getByRole('alert').filter({ hasText: 'Nobody answered the Windows permission prompt in time' })
    await alert.getByRole('button', { name: 'Continue without the administrator parts' }).click()
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })
    await expect(page.getByText(/players outside this computer cannot connect yet/)).toBeVisible()
  })

  test('signed in from another device: Windows asks only when the person says so', async ({ page, open }) => {
    await open('/install', { scenario: 'windows' })
    await toSummary(page, 'remote-town')
    await installButton(page).click()
    const alert = page.getByRole('alert').filter({ hasText: 'You are signed in from another device' })
    await expect(alert).toBeVisible({ timeout: 20_000 })
    await alert.getByRole('button', { name: 'Ask for administrator permission' }).click()
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })
  })
})
