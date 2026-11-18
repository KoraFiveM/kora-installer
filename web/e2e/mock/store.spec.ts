import { test, expect } from '../fixtures'

test.describe('Recent jobs (kept across a restart)', () => {
  test('shows what ran before the restart, with what to do for the one that was interrupted', async ({ page, open, scan }) => {
    await open('/settings', { scenario: 'restarted' })
    await expect(page.getByText('Stopped by a restart').first()).toBeVisible()
    const rows = page.getByTestId('job-row')
    await expect(rows).toHaveCount(4)
    // the interrupted install carries its guided problem and a way to carry on
    const install = rows.filter({ hasText: 'Installed a server' })
    await expect(install.getByText(/did not finish. Kora kept your answers: open Install and choose Carry on/)).toBeVisible()
    // the interrupted server job says what to look at, and offers nothing it cannot do
    const control = rows.filter({ hasText: 'Started, stopped or restarted a server' })
    await expect(control.getByText(/Look at the server's state, then run it again/)).toBeVisible()
    await expect(control.getByRole('link')).toHaveCount(0)
    // finished and failed ones are there too
    await expect(rows.getByText('Finished', { exact: true })).toHaveCount(1)
    await expect(rows.filter({ hasText: 'The update could not be downloaded' })).toHaveCount(1)
    await scan('settings: recent jobs after a restart')
  })

  test('"Open Install to carry on" leads to the install page, which offers Carry on for that install', async ({ page, open, scan }) => {
    await open('/settings', { scenario: 'restarted' })
    await page.getByRole('link', { name: 'Open Install to carry on' }).click()
    await expect(page).toHaveURL(/\/install/)
    await expect(page.getByRole('button', { name: 'Carry on' })).toBeVisible()
    await expect(page.getByText(/Staging/).first()).toBeVisible()
    await scan('install page offering to carry on after a restart')
  })

  test('a panel that has not been restarted says nothing has run yet', async ({ page, open }) => {
    await open('/settings')
    await expect(page.getByText('Nothing has run yet.')).toBeVisible()
  })
})

test.describe('Activity log (administrators only)', () => {
  test('an administrator sees who did what, can filter, and load older entries', async ({ page, open, scan }) => {
    await open('/settings', { scenario: 'restarted' })
    await expect(page.getByRole('heading', { name: 'Activity log' })).toBeVisible()
    const rows = page.getByTestId('audit-row')
    await expect(rows.first()).toBeVisible()
    await expect(page.getByText('sam@example.com (signed in from another device with tailscale, admin)').first()).toBeVisible()
    await expect(page.getByText('demo (on this computer)').first()).toBeVisible()
    await expect(page.getByText(/Passwords, licence keys and what was typed are never recorded/)).toBeVisible()
    const first = await rows.count()
    expect(first).toBe(50)
    await page.getByRole('button', { name: 'Show older entries' }).click()
    await expect.poll(() => rows.count()).toBeGreaterThan(first)
    await expect(page.getByText('demo (a kora command)').first()).toBeVisible()
    await scan('settings: activity log')
    await page.getByLabel('Result').selectOption('refused')
    await expect(rows).toHaveCount(1)
    await expect(rows.first()).toContainText('Deleted a server')
    await expect(rows.first()).toContainText('Not allowed')
  })

  test('with the administrator service its own entries are in the same table, labelled and merged by time', async ({ page, open, scan }) => {
    await open('/settings', { scenario: 'service' })
    await expect(page.getByRole('heading', { name: 'Activity log' })).toBeVisible()
    const service = page.locator('[data-source="service"]')
    await expect(service).toHaveCount(4)
    await expect(service.first()).toContainText('Set up scheduled backups as an operating-system job')
    await expect(service.first()).toContainText('Done with administrator rights')
    await expect(service.nth(2)).toContainText('code 1')
    await expect(page.getByText(/only what it did for your account is shown/)).toBeVisible()
    await scan('settings: activity log with the administrator service entries')
  })

  test('without the administrator service the log says nothing was done with administrator rights', async ({ page, open }) => {
    await open('/settings', { scenario: 'restarted' })
    await expect(page.getByTestId('service-audit-note')).toContainText('nothing has been done with administrator rights')
    await expect(page.locator('[data-source="service"]')).toHaveCount(0)
  })

  test('a remote operator does not see it (and the page never asks for it)', async ({ page, open }) => {
    const asked: string[] = []
    page.on('request', (r) => asked.push(new URL(r.url()).pathname))
    await open('/settings', { scenario: 'restarted', as: 'remote-operator' })
    await expect(page.getByRole('heading', { name: 'Your access' })).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Activity log' })).toHaveCount(0)
    expect(asked.filter((p) => p.endsWith('/audit'))).toEqual([])
    // recent jobs are theirs to see
    await expect(page.getByRole('heading', { name: 'Recent jobs' })).toBeVisible()
  })

  test('a local operator does not see it either', async ({ page, open }) => {
    await open('/settings', { as: 'operator' })
    await expect(page.getByRole('heading', { name: 'Recent jobs' })).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Activity log' })).toHaveCount(0)
  })
})
