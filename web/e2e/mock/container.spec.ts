import { test, expect, KEY } from '../fixtures'

// Container mode (Linux, Podman or Docker): the install wizard asks how the server runs when the System check
// found an engine that works, and a container server is labelled wherever servers are listed.

const next = (page: import('@playwright/test').Page) => page.getByRole('button', { name: 'Next' })

test.describe('container mode', () => {
  test('where no container engine works, the wizard does not ask and nothing says container', async ({ page, open }) => {
    await open('/install')
    await page.getByLabel('Licence key', { exact: true }).fill(KEY)
    await next(page).click()
    await next(page).click()
    await page.getByRole('button', { name: 'Choose everything myself' }).click()
    await expect(page.getByText('How the server runs', { exact: true })).toHaveCount(0)
    await page.goto('/system?scenario=default')
    await expect(page.getByRole('heading', { name: 'Background service' })).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Containers' })).toHaveCount(0)
  })

  test('choose a container: no service, a MariaDB container, install, and the list says "In a container"', async ({ page, open, scan }) => {
    await open('/install', { scenario: 'containers' })
    await page.getByLabel('Licence key', { exact: true }).fill(KEY)
    await next(page).click()
    await page.getByLabel('Server name').fill('box-night')
    await next(page).click()
    await page.getByRole('button', { name: 'Choose everything myself' }).click()
    // walk the steps until the run-mode question
    for (let i = 0; i < 10 && (await page.getByText('How should this server run?').count()) === 0; i++) await next(page).click()
    await expect(page.getByText('How should this server run?')).toBeVisible()
    await expect(page.getByText(/Podman 4\.9\.3, rootless: ready to run servers/)).toBeVisible()
    await expect(page.getByLabel(/As a system service/)).toBeChecked()
    await page.getByLabel(/In a container/).check()
    await expect(page.getByText('What a container does and does not protect')).toBeVisible()
    await scan('wizard: how the server runs')

    await next(page).click()
    await expect(page.getByLabel(/A MariaDB container next to the server/)).toBeChecked()
    await expect(page.getByText('Database administrator login')).toHaveCount(0)
    await scan('wizard: database in a container')
    await next(page).click()
    await expect(page.getByText(/There is no background service to install/)).toBeVisible()
    for (let i = 0; i < 6 && (await page.getByRole('button', { name: 'Install the server' }).count()) === 0; i++) await next(page).click()
    await expect(page.getByText('In a container', { exact: true })).toBeVisible()
    await expect(page.getByText('A MariaDB container next to the server')).toBeVisible()
    await page.getByRole('button', { name: 'Install the server' }).click()
    // the container steps are part of the progress
    await expect(page.getByText('Creating the server\'s container').first()).toBeVisible({ timeout: 40_000 })
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })

    await page.getByRole('link', { name: 'Servers' }).click()
    await expect(page.getByRole('link', { name: 'box-night' })).toBeVisible()
    await expect(page.getByText('In a container (podman, rootless)')).toHaveCount(2)
    await scan('servers list with container servers')
  })

  test('the server page and the System check name the engine', async ({ page, open }) => {
    await open('/servers/kora-live', { scenario: 'containers' })
    await expect(page.getByRole('heading', { name: 'kora-live' })).toBeVisible()
    await expect(page.getByText('In a container (podman, rootless)').first()).toBeVisible()
    await page.goto('/system?scenario=containers')
    await expect(page.getByRole('heading', { name: 'Containers' })).toBeVisible()
    await expect(page.getByText(/ready to run servers/)).toBeVisible()
  })
})
