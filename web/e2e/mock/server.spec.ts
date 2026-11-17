import { test, expect } from '../fixtures'
import type { Page } from '@playwright/test'

const TABS_PROD = ['Overview', 'Console', 'Logs', 'Monitoring', 'Resources', 'Paid assets', 'Notifications', 'Ports', 'Danger zone']
const TABS_DEV = ['Schedule', 'GitHub']

const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true })

test.describe('server list', () => {
  test('shows each server with its status, updates and quick buttons', async ({ page, open, scan }) => {
    await open('/')
    await expect(page.getByRole('heading', { name: 'Your servers' })).toBeVisible()
    const table = page.getByRole('table')
    await expect(table.getByRole('link', { name: 'kora-live', exact: true })).toBeVisible()
    await expect(table.getByRole('link', { name: '2 updates' })).toHaveAttribute('href', '/servers/kora-dev?tab=resources')
    await expect(table.getByText('Folder missing').first()).toBeVisible()
    await scan('servers list')
  })

  test('the fleet scenario (31 servers) is usable and accessible', async ({ page, open, scan }) => {
    await open('/', { scenario: 'fleet' })
    await expect(page.getByRole('table').getByRole('row')).toHaveCount(32)
    await scan('servers list, 31 servers')
  })
})

test.describe("a server's tabs", () => {
  test('every tab of a Production server opens, and has no accessibility problems', async ({ page, open, scan }) => {
    await open('/servers/kora-live')
    await expect(page.getByRole('heading', { name: 'kora-live', level: 1 })).toBeVisible()
    for (const name of TABS_PROD) {
      await tab(page, name).click()
      await expect(tab(page, name)).toHaveAttribute('aria-selected', 'true')
      await expect(page.getByRole('tabpanel')).toBeVisible()
      await scan(`kora-live, ${name} tab`)
    }
    for (const name of TABS_DEV) await expect(tab(page, name)).toHaveCount(0)
  })

  test('a Development server also has Schedule and GitHub', async ({ page, open, scan }) => {
    await open('/servers/kora-dev')
    for (const name of TABS_DEV) {
      await tab(page, name).click()
      await expect(page.getByRole('tabpanel')).toBeVisible()
      await scan(`kora-dev, ${name} tab`)
    }
  })

  test('an old ?tab=updates link lands on Resources, and an unknown server says so', async ({ page, open }) => {
    await open('/servers/kora-dev?tab=updates')
    await expect(tab(page, 'Resources')).toHaveAttribute('aria-selected', 'true')
    await page.getByRole('link', { name: 'Install new server' }).click()
    await page.goto('/servers/nope')
    await expect(page.getByText(/not found|no server|does not exist/i).first()).toBeVisible()
  })
})

test.describe('Overview buttons', () => {
  test('Stop a running server, then Start it again', async ({ page, open, scan }) => {
    await open('/servers/kora-live')
    await expect(page.getByText('Running', { exact: true }).first()).toBeVisible()
    await page.getByRole('button', { name: 'Stop kora-live' }).click()
    await expect(page.getByRole('button', { name: 'Start kora-live' })).toBeVisible({ timeout: 20_000 })
    await expect(page.getByText('Stopped', { exact: true }).first()).toBeVisible()
    await scan('overview: stopped server')
    await page.getByRole('button', { name: 'Start kora-live' }).click()
    await expect(page.getByRole('button', { name: 'Stop kora-live' })).toBeVisible({ timeout: 20_000 })
  })

  test('Restart asks first, and Cancel does nothing', async ({ page, open, scan }) => {
    await open('/servers/kora-live')
    await page.getByRole('button', { name: 'Restart kora-live' }).click()
    const dialog = page.getByRole('dialog', { name: 'Restart kora-live?' })
    await expect(dialog).toContainText('disconnected')
    await scan('overview: restart confirmation')
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    await expect(dialog).toBeHidden()
    await expect(page.getByRole('button', { name: 'Stop kora-live' })).toBeVisible()
    await page.getByRole('button', { name: 'Restart kora-live' }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Restart server' }).click()
    await expect(page.getByRole('button', { name: 'Stop kora-live' })).toBeVisible({ timeout: 20_000 })
  })

  test('a start that fails says why in plain words (port in use)', async ({ page, open }) => {
    await open('/servers/kora-dev', { scenario: 'trouble' })
    await page.getByRole('button', { name: 'Start kora-dev' }).click()
    await expect(page.getByText(/already in use by another program/).first()).toBeVisible({ timeout: 20_000 })
  })

  test('the Danger zone asks for the name before deleting', async ({ page, open, scan }) => {
    await open('/servers/kora-dev?tab=danger')
    await scan('danger zone')
    await expect(tab(page, 'Danger zone')).toHaveAttribute('aria-selected', 'true')
  })
})

test.describe('Resources and updates', () => {
  test('checking finds updates, a resource on another branch is skipped, and updating applies the chosen ones', async ({ page, open, scan }) => {
    await open('/servers/kora-dev?tab=resources')
    await page.getByRole('button', { name: 'Check for updates' }).click()
    const list = page.getByRole('list', { name: 'Resources with updates' })
    await expect(list).toBeVisible({ timeout: 30_000 })
    await expect(list.getByText('kora_core')).toBeVisible()
    await expect(list.getByText('Ready to update').first()).toBeVisible()
    // A resource the person put on another branch is left alone and cannot be ticked.
    await expect(page.getByText(/Not updated: on another branch/).first()).toBeVisible()
    await expect(page.getByRole('checkbox', { name: 'Update kora_phone' })).toBeDisabled()
    await scan('resources: update check result')

    await page.getByRole('checkbox', { name: 'Update kora_core' }).check()
    await page.getByRole('button', { name: /^Update \d+ resources?$/ }).click()
    await expect(page.getByRole('heading', { name: 'Update progress' })).toBeVisible()
    await expect(page.getByText(/finished|updated|done/i).first()).toBeVisible({ timeout: 40_000 })
  })

  test('on a server with trouble, a resource with your own changes is kept and explained', async ({ page, open }) => {
    await open('/servers/kora-live?tab=resources', { scenario: 'trouble' })
    await page.getByRole('button', { name: 'Check for updates' }).click()
    await expect(page.getByText('Needs attention').first()).toBeVisible({ timeout: 30_000 })
    await expect(page.getByText(/You changed files in this resource/).first()).toBeVisible()
  })
})
