import { test, expect, toSummary } from '../fixtures'

const installButton = (page: import('@playwright/test').Page) => page.getByRole('button', { name: 'Install the server' })
const done = (page: import('@playwright/test').Page) => expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })

// Kora's administrator service: when it is there the administrator parts of an install just happen (no prompt, no
// notice to approve); when it is not (a per-user install) there is no automatic path, only choices.

test.describe('install with the administrator service', () => {
  test('service ready: the wizard says the service does those parts and there is nothing to approve', async ({ page, open, scan }) => {
    await open('/install', { scenario: 'service' })
    await toSummary(page, 'my-town')
    const notice = page.getByRole('status').filter({ hasText: "Kora's administrator service will do these parts" })
    await expect(notice).toContainText('There is nothing to approve')
    await expect(notice).toContainText('Open the game port in the firewall')
    await expect(page.getByText(/Windows will ask/)).toHaveCount(0)
    await scan('wizard: administrator service ready')
  })

  test('service ready: the install just runs, with no permission step', async ({ page, open }) => {
    await open('/install', { scenario: 'service' })
    await toSummary(page, 'my-town')
    await installButton(page).click()
    await done(page)
    await expect(page.getByText('Administrator permission')).toHaveCount(0)
  })

  test('service ready, signed in from another device: the same, nothing waits for a desktop', async ({ page, open }) => {
    await open('/install', { scenario: 'service', as: 'remote-admin' })
    await toSummary(page, 'remote-town')
    await installButton(page).click()
    await done(page)
  })
})

test.describe('install without the administrator service', () => {
  test('says the service is not installed, and "Install without these parts" turns them off before anything starts', async ({ page, open, scan }) => {
    await open('/install', { scenario: 'noservice' })
    await toSummary(page, 'my-town')
    await expect(page.getByText("Kora's administrator service is not installed")).toBeVisible()
    await scan('wizard: administrator service missing')
    await page.getByRole('button', { name: 'Install without these parts' }).click()
    await expect(page.getByText("Kora's administrator service is not installed")).toHaveCount(0)
    await expect(page.getByText('Leave closed')).toBeVisible()
    await expect(installButton(page)).toBeEnabled()
  })

  test('Linux: the install stops at the start with the choices; the command is shown, there is no button that could launch the installer', async ({ page, open, scan }) => {
    await open('/install', { scenario: 'noservice' })
    await toSummary(page, 'my-town')
    await installButton(page).click()
    const alert = page.getByRole('alert').filter({ hasText: 'its administrator service is not installed on this computer' })
    await expect(alert).toBeVisible({ timeout: 20_000 })
    await expect(alert.getByRole('button', { name: 'Continue without the administrator parts' })).toBeVisible()
    await expect(alert.getByRole('button', { name: "Install Kora's administrator service" })).toHaveCount(0)
    await expect(alert.getByText(/kora service install/).first()).toBeVisible()
    await scan('wizard: service missing on Linux')
  })

  test('Windows: "Install Kora\'s administrator service" runs the installer and the install carries on by itself', async ({ page, open, scan }) => {
    await open('/install', { scenario: 'windows' })
    await toSummary(page, 'my-town')
    await installButton(page).click()
    const alert = page.getByRole('alert').filter({ hasText: 'its administrator service is not installed on this computer' })
    await expect(alert.getByText('Show me the command')).toBeVisible({ timeout: 20_000 })
    await expect(alert.getByRole('button', { name: 'Continue without the administrator parts' })).toBeVisible()
    await scan('wizard on Windows: service missing')
    await alert.getByRole('button', { name: "Install Kora's administrator service" }).click()
    await done(page)
  })

  test('"Continue without the administrator parts" carries on as the person and says what is missing', async ({ page, open }) => {
    await open('/install', { scenario: 'windows' })
    await toSummary(page, 'my-town')
    await installButton(page).click()
    const alert = page.getByRole('alert').filter({ hasText: 'its administrator service is not installed on this computer' })
    await alert.getByRole('button', { name: 'Continue without the administrator parts' }).click()
    // the answers change (no firewall, no background service); the person starts the install again
    await expect(page.getByText('Leave closed')).toBeVisible()
    await installButton(page).click()
    await done(page)
    await expect(page.getByText(/players outside this computer cannot connect yet/)).toBeVisible()
  })

  test('signed in from another device without the service: the same choices (the installer is only launched on this computer)', async ({ page, open }) => {
    await open('/install', { scenario: 'noservice', as: 'remote-admin' })
    await toSummary(page, 'remote-town')
    await installButton(page).click()
    const alert = page.getByRole('alert').filter({ hasText: 'its administrator service is not installed on this computer' })
    await expect(alert.getByRole('button', { name: 'Continue without the administrator parts' })).toBeVisible({ timeout: 20_000 })
  })
})

test.describe('the service is installed but not for this account', () => {
  test('the message says who can fix it and gives the command', async ({ page, open, scan }) => {
    await open('/install', { scenario: 'notallowed' })
    await toSummary(page, 'my-town')
    await installButton(page).click()
    const alert = page.getByRole('alert').filter({ hasText: 'not turned on for your account' })
    await expect(alert).toBeVisible({ timeout: 20_000 })
    await expect(alert.getByRole('button', { name: 'Continue without the administrator parts' })).toBeVisible()
    await expect(alert.getByRole('button', { name: "Install Kora's administrator service" })).toHaveCount(0)
    await expect(alert.getByText(/kora service allow/).first()).toBeVisible()
    await scan('wizard: not allowed')
  })
})
