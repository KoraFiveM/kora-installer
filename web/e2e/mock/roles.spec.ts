import { test, expect, typeKeyIfAsked } from '../fixtures'

const WHY = 'Your account (operator) cannot do this. Ask an administrator for admin access.'

test.describe('a remote operator (Tailscale)', () => {
  test('can look and run servers, but is told why installing and adding are off', async ({ page, open, scan }) => {
    await open('/', { as: 'remote-operator' })
    await expect(page.getByRole('table')).toBeVisible()
    // The install link is not a link: it says why.
    await expect(page.getByRole('link', { name: 'Install new server' })).toHaveCount(0)
    await expect(page.getByText('Needs an administrator account.')).toBeVisible()
    // The Fleet page keeps other panels' tokens on this computer only.
    await expect(page.getByRole('link', { name: 'Fleet' })).toHaveCount(0)
    await scan('servers list as a remote operator')
    await page.getByRole('button', { name: 'Add an existing server' }).click()
    const dialog = page.getByRole('dialog')
    await expect(dialog.getByRole('button', { name: 'Add this folder' })).toBeDisabled()
    await expect(dialog.getByText(WHY).first()).toBeVisible()
    await scan('add an existing server as an operator')
  })

  test('the install page itself explains instead of showing the steps', async ({ page, open, scan }) => {
    await open('/install', { as: 'remote-operator' })
    await expect(page.getByRole('alert').filter({ hasText: WHY })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Next' })).toHaveCount(0)
    await scan('install page as an operator')
  })

  test('Settings shows who they are and what that allows, not the remote-access switches', async ({ page, open, scan }) => {
    await open('/settings', { as: 'remote-operator' })
    await expect(page.getByRole('heading', { name: 'Your access' })).toBeVisible()
    await expect(page.getByText('Allow remote access')).toHaveCount(0)
    await expect(page.getByText('kim')).toBeVisible()
    await scan('settings as a remote operator')
  })

  test('the licence keys can be seen but not managed', async ({ page, open }) => {
    await open('/settings', { as: 'remote-operator' })
    await expect(page.getByText(/Only an administrator can add, rename, replace or delete keys/)).toBeVisible()
    await expect(page.getByRole('button', { name: 'Save key' })).toHaveCount(0)
  })

  test('server buttons are available to an operator (start / stop)', async ({ page, open }) => {
    await open('/servers/kora-live', { as: 'remote-operator' })
    await expect(page.getByRole('button', { name: 'Stop kora-live' })).toBeEnabled()
  })

  test('a viewer-level reason is shown for the Danger zone', async ({ page, open, scan }) => {
    await open('/servers/kora-dev?tab=danger', { as: 'remote-operator' })
    await expect(page.getByRole('tabpanel')).toBeVisible()
    await scan('danger zone as a remote operator')
  })
})

test.describe('a local operator', () => {
  test('cannot install, sees the same reason', async ({ page, open }) => {
    await open('/install', { as: 'operator' })
    await expect(page.getByRole('alert').filter({ hasText: WHY })).toBeVisible()
  })
})

test.describe('a remote administrator with a token', () => {
  test('can install (the wizard opens), but deleting waits for the remote-actions switch', async ({ page, open, scan }) => {
    await open('/install', { as: 'remote-admin' })
    await expect(page.getByRole('heading', { name: 'Your licence key' })).toBeVisible()
    await scan('wizard as a token administrator')
    await page.goto('/servers/kora-dev?tab=danger')
    await expect(page.getByRole('tab', { name: 'Danger zone' })).toHaveAttribute('aria-selected', 'true')
    await expect(page.getByText(/kora panel remote-actions on/).first()).toBeVisible()
    await scan('danger zone as a token administrator')
  })

  test('Settings explains what a token sign-in cannot do', async ({ page, open }) => {
    await open('/settings', { as: 'remote-admin' })
    await expect(page.getByText(/turned off for sign-ins that do not come through Tailscale/).first()).toBeVisible()
    await expect(page.getByText('Uninstall Kora')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Show what would be removed' })).toBeDisabled()
  })

  test('can finish a whole install from another device', async ({ page, open }) => {
    await open('/install', { as: 'remote-admin' })
    await typeKeyIfAsked(page, 'cfxk_abcdefghij_12345')
    await page.getByRole('button', { name: 'Next' }).click()
    await page.getByRole('button', { name: 'Next' }).click()
    await expect(page.getByRole('button', { name: 'Install the server' })).toBeEnabled()
  })
})
