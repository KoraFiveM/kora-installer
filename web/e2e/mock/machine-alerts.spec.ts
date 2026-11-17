// Alerts that belong to no server (the panel's HTTPS certificate, Kora's administrator service) in the alert list at the top of
// every page: they say what is wrong, open Settings, and can be dismissed like any other alert.
import { test, expect } from '../fixtures'

test.describe('Machine-level alerts', () => {
  test('a certificate alert has no server, opens the page that says what to do and is dismissed like the others', async ({ page, open, scan }) => {
    await open('/')
    const banner = page.getByLabel('Server alerts')
    await expect(banner.getByText(/HTTPS certificate ending soon/)).toBeVisible()
    await expect(banner.getByText(/expires in 9 days/)).toBeVisible()
    await scan('the alert list with a certificate alert')
    await banner.getByRole('link', { name: 'What to do' }).click()
    await expect(page).toHaveURL(/\/settings$/)
    await expect(page.getByRole('heading', { name: 'Activity log' })).toBeVisible()
    await page.getByRole('button', { name: 'Dismiss the alert: HTTPS certificate ending soon' }).click()
    await expect(page.getByText(/expires in 9 days/)).toHaveCount(0)
  })

  test('a server alert still names its server', async ({ page, open }) => {
    await open('/')
    await expect(page.getByLabel('Server alerts').getByRole('link', { name: 'kora-dev' })).toBeVisible()
  })
})
