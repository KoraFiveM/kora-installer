// "Can players reach this server?" and "Open txAdmin" in a real browser, against the mock backend.
import { test, expect } from '../fixtures'
import type { Page } from '@playwright/test'

const layer = (page: Page, id: string) => page.locator(`[data-layer="${id}"]`)

test.describe('Can players reach this server?', () => {
  test('says what it looks at, contacts nothing before asked, then shows one line per layer in each state', async ({ page, open, scan }) => {
    await open('/servers/kora-live')
    await expect(page.getByText('It only looks at this computer and your router. Nothing outside is contacted unless you allow it.')).toBeVisible()
    await scan('overview with the reach card before a check')
    await page.getByRole('button', { name: 'Check now' }).click()
    await expect(layer(page, 'running')).toHaveAttribute('data-state', 'ok')
    await expect(layer(page, 'firewall')).toHaveAttribute('data-state', 'problem')
    await expect(layer(page, 'router')).toHaveAttribute('data-state', 'unknown')
    await expect(layer(page, 'internet')).toHaveAttribute('data-state', 'unknown')
    // The router line gives exact values to type, and a button for what Kora can do.
    await expect(layer(page, 'router').getByText(/outside\/external port 30120, inside\/internal port 30120, destination address 192\.168\.1\.50/)).toBeVisible()
    await expect(page.getByRole('button', { name: /Forward port 30120 for me/ })).toBeVisible()
    await expect(page.getByLabel('Address on your own network')).toContainText('192.168.1.50:30120')
    await scan('overview with a reach check')
  })

  test('the firewall fix opens the port and the line turns OK', async ({ page, open }) => {
    await open('/servers/kora-live')
    await page.getByRole('button', { name: 'Check now' }).click()
    await page.getByRole('button', { name: 'Open port 30120 in the firewall' }).click()
    await expect(layer(page, 'firewall')).toHaveAttribute('data-state', 'ok')
  })

  test('an outside lookup needs a yes first: it says what is contacted and what it learns, then remembers the choice', async ({ page, open, scan }) => {
    await open('/servers/kora-live')
    await page.getByRole('button', { name: 'Check now' }).click()
    await page.getByRole('button', { name: /Look my server up on Cfx\.re/ }).click()
    const ask = page.getByRole('status').filter({ hasText: 'This will contact' })
    await expect(ask).toContainText("Cfx.re's public server list (frontend.cfx-services.net)")
    await expect(ask).toContainText('Cfx.re already knows it')
    await scan('the consent step before an outside lookup')
    // Nothing was contacted yet.
    await expect(page.getByText(/This check asked/)).toHaveCount(0)
    await ask.getByRole('button', { name: 'Allow and check again' }).click()
    await expect(layer(page, 'internet')).toHaveAttribute('data-state', 'likely')
    await expect(page.getByText(/Probably OK/).first()).toBeVisible()
    await expect(page.getByText(/This check asked Cfx\.re's public server list/)).toBeVisible()
    await expect(page.getByLabel('Cfx.re join link').first()).toContainText('https://cfx.re/join/abc123')
    await expect(page.getByLabel("Cfx.re's public server list")).toBeChecked()
  })

  test('"Not now" asks nothing; "Just this once" looks it up without saving', async ({ page, open }) => {
    await open('/servers/kora-live')
    await page.getByRole('button', { name: 'Check now' }).click()
    await page.getByRole('button', { name: /Look my server up on Cfx\.re/ }).click()
    await page.getByRole('button', { name: 'Not now' }).click()
    await expect(page.getByRole('status').filter({ hasText: 'This will contact' })).toHaveCount(0)
    await page.getByRole('button', { name: /Look my server up on Cfx\.re/ }).click()
    await page.getByRole('button', { name: 'Just this once' }).click()
    await expect(layer(page, 'internet')).toHaveAttribute('data-state', 'likely')
    await expect(page.getByLabel("Cfx.re's public server list")).not.toBeChecked()
  })

  test('behind a provider-shared address: explains, offers no forwarding and no public address', async ({ page, open, scan }) => {
    await open('/servers/adopted-existing-server', { scenario: 'fleet' })
    await page.getByRole('button', { name: 'Check now' }).click()
    await expect(layer(page, 'router')).toHaveAttribute('data-state', 'problem')
    await expect(layer(page, 'router').getByText(/carrier-grade NAT/).first()).toBeVisible()
    await expect(layer(page, 'router').getByText(/Tailscale or ZeroTier/)).toBeVisible()
    await expect(page.getByRole('button', { name: /Forward port/ })).toHaveCount(0)
    await expect(page.getByLabel('Address on the internet')).toHaveCount(0)
    await scan('reach card behind carrier-grade NAT')
  })

  test('turning port forwarding on asks a second time with the risk, and can be turned off again', async ({ page, open }) => {
    await open('/servers/kora-live')
    await page.getByRole('button', { name: 'Check now' }).click()
    await page.getByRole('button', { name: /Forward port 30120 for me/ }).click()
    const ask = page.getByRole('alert').filter({ hasText: 'Let Kora change your router?' })
    await expect(ask).toContainText("game port only")
    await ask.getByRole('button', { name: 'Not now' }).click()
    await expect(layer(page, 'router')).toHaveAttribute('data-state', 'unknown')
    await page.getByRole('button', { name: /Forward port 30120 for me/ }).click()
    await page.getByRole('button', { name: 'Turn on port forwarding' }).click()
    await expect(layer(page, 'router')).toHaveAttribute('data-state', 'ok')
    await expect(page.getByText('Automatic port forwarding is on.')).toBeVisible()
    await page.getByRole('button', { name: 'Turn it off' }).click()
    await expect(layer(page, 'router')).toHaveAttribute('data-state', 'unknown')
    await expect(page.getByText('Automatic port forwarding is on.')).toHaveCount(0)
  })

  test('a stopped server offers to start it from the card', async ({ page, open }) => {
    await open('/servers/kora-dev')
    await page.getByRole('button', { name: 'Check now' }).click()
    await expect(layer(page, 'running')).toHaveAttribute('data-state', 'problem')
    await page.getByRole('button', { name: 'Start the server' }).click()
  })

  test('an operator can check but not change the router', async ({ page, open }) => {
    await open('/servers/kora-live', { as: 'operator' })
    await page.getByRole('button', { name: 'Check now' }).click()
    await expect(page.getByRole('button', { name: /Forward port 30120 for me/ })).toBeDisabled()
  })
})

test.describe('Open txAdmin', () => {
  test('at the computer that runs Kora it is a plain link to localhost', async ({ page, open }) => {
    await open('/servers/kora-live')
    const link = page.getByRole('link', { name: 'Open txAdmin' }).first()
    await expect(link).toHaveAttribute('href', 'http://localhost:40120')
  })

  test('from another computer a Tailscale administrator gets a one-time ticket and lands on the gateway address', async ({ page, context, open, scan }) => {
    await context.route('**/__kora/enter**', (route) => route.fulfill({ status: 200, contentType: 'text/html', body: '<h1>txAdmin sign-in</h1>' }))
    await open('/servers/kora-live', { as: 'remote-tailscale-admin' })
    await expect(page.getByRole('link', { name: 'Open txAdmin' })).toHaveCount(0)
    const button = page.getByRole('button', { name: 'Open txAdmin' }).first()
    await expect(button).toBeEnabled()
    await scan('overview as a remote administrator')
    const popup = page.waitForEvent('popup')
    await button.click()
    const tab = await popup
    await tab.waitForURL(/:50120\/__kora\/enter\?t=demo$/)
    // txAdmin has its own sign-in, and the panel says which account.
    await expect(page.getByText(/use the account/).first()).toContainText('kora-admin')
    await expect(page.getByText(/cannot sign you in/).first()).toBeVisible()
  })

  test('an operator sees no button at all', async ({ page, open }) => {
    await open('/servers/kora-live', { as: 'remote-operator' })
    await expect(page.getByRole('heading', { name: 'kora-live', level: 1 })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Open txAdmin' })).toHaveCount(0)
    await expect(page.getByRole('link', { name: 'Open txAdmin' })).toHaveCount(0)
  })

  test('a token administrator is refused in words, with the command that turns remote actions on', async ({ page, open }) => {
    await open('/servers/kora-live', { as: 'remote-admin' })
    await expect(page.getByRole('button', { name: 'Open txAdmin' }).first()).toBeDisabled()
    await expect(page.getByText(/kora panel remote-actions on/).first()).toBeVisible()
  })
})
