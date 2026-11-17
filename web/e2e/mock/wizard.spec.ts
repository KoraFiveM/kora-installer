import { test, expect, toSummary } from '../fixtures'

const installButton = (page: import('@playwright/test').Page) => page.getByRole('button', { name: 'Install the server' })
const next = (page: import('@playwright/test').Page) => page.getByRole('button', { name: 'Next' })

test.describe('install wizard: the happy paths', () => {
  test('a Production server: licence key, name, summary, install, finished page', async ({ page, open, scan }) => {
    await open('/install')
    await expect(page.getByRole('heading', { name: 'Your licence key' })).toBeVisible()
    await expect(next(page)).toBeDisabled()
    await scan('wizard: licence key')
    await toSummary(page, 'friday-night')
    await expect(page.getByText('Production (a server for players)')).toBeVisible()
    await expect(page.getByText('/srv/fivem/friday-night')).toBeVisible()
    await scan('wizard: summary')

    await installButton(page).click()
    // Progress is shown while it runs; the finished page follows.
    await expect(page.getByRole('progressbar').first()).toBeVisible()
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })
    await expect(page.getByText(/address on your own network/)).toBeVisible()
    await scan('wizard: all done')

    // The new server is in the list, running.
    await page.getByRole('link', { name: 'Servers' }).click()
    await expect(page.getByRole('link', { name: 'friday-night' })).toBeVisible()
  })

  test('a Development server adds the GitHub step, which needs the sign-in first', async ({ page, open, scan }) => {
    await open('/install')
    await page.getByLabel('Licence key', { exact: true }).fill('cfxk_abcdefghij_12345')
    await next(page).click()
    await page.getByRole('button', { name: 'Make a development server instead' }).click()
    await page.getByLabel(/Development/).check()
    await expect(page.getByText('3. GitHub')).toBeVisible()
    await next(page).click()
    await expect(page.getByRole('heading', { name: 'GitHub' })).toBeVisible()
    await expect(next(page)).toBeDisabled()
    await expect(page.getByText('Sign in to GitHub to continue.')).toBeVisible()
    await scan('wizard: GitHub step, signed out')

    await page.getByRole('button', { name: 'Sign in to GitHub' }).click()
    // The one-time code is shown, and the page notices the approval by itself.
    await expect(page.getByText('Finish signing in on GitHub')).toBeVisible()
    await expect(page.getByText('Signed in', { exact: true })).toBeVisible({ timeout: 30_000 })
    // The forks go into an account: pick the first one offered.
    const owner = page.getByLabel('Put the forks in')
    await expect(owner.locator('option')).not.toHaveCount(1)
    await owner.selectOption({ index: 1 })
    await scan('wizard: GitHub step, signed in')
    await expect(next(page)).toBeEnabled()
    await next(page).click()
    await expect(page.getByRole('heading', { name: 'Check and install' })).toBeVisible()
    await expect(page.getByText('Development (for building and changing resources)', { exact: false })).toBeVisible()
    await expect(installButton(page)).toBeEnabled()
    await installButton(page).click()
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })
  })

  test('"Choose everything myself" walks every step and "Change" returns to the summary', async ({ page, open, scan }) => {
    await open('/install')
    await page.getByRole('button', { name: 'Choose everything myself' }).click()
    for (const title of ['Kind of server', 'Name your server', 'Ports', 'FiveM server build', 'Resources', 'Database', 'Options']) {
      await expect(page.getByRole('heading', { name: title })).toBeVisible()
      if (title === 'Ports') await expect(page.getByText(/is free/).first()).toBeVisible()
      await scan(`wizard step: ${title}`)
      await next(page).click()
    }
    await page.getByLabel('Licence key', { exact: true }).fill('cfxk_abcdefghij_12345')
    await next(page).click()
    await expect(page.getByRole('heading', { name: 'Your Cfx.re account' })).toBeVisible()
    await scan('wizard step: Cfx.re account')
    await next(page).click()
    await expect(page.getByRole('heading', { name: 'Check and install' })).toBeVisible()
    await page.getByRole('button', { name: 'Change Database', exact: true }).click()
    await page.getByLabel(/Skip the database/).check()
    await page.getByRole('button', { name: 'Back to the summary' }).click()
    await expect(page.getByText('Not needed')).toBeVisible()
    await expect(installButton(page)).toBeEnabled()
  })
})

test.describe('install wizard: when a step fails', () => {
  test('retry: says what happened, hides the technical part, retries only that step', async ({ page, open, scan }) => {
    await open('/install')
    await toSummary(page, 'fail-town')
    await installButton(page).click()
    await expect(page.getByRole('heading', { name: 'The install stopped' })).toBeVisible({ timeout: 40_000 })
    const alert = page.getByRole('alert').filter({ hasText: 'Could not log in to the database server' })
    await expect(alert).toBeVisible()
    await expect(alert.getByRole('button', { name: 'Change the database choice' })).toBeVisible()
    await expect(alert.getByRole('button', { name: 'Skip this step' })).toHaveCount(0)
    await expect(page.getByText(/What was kept/)).toBeVisible()
    await scan('wizard: failed step')
    await alert.getByRole('button', { name: 'Try this step again' }).click()
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })
  })

  test('skip: a step that is safe to skip can be skipped, and the install finishes', async ({ page, open }) => {
    await open('/install')
    await toSummary(page, 'skip-town')
    await installButton(page).click()
    await expect(page.getByRole('heading', { name: 'The install stopped' })).toBeVisible({ timeout: 40_000 })
    await page.getByRole('alert').getByRole('button', { name: 'Skip this step' }).click()
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 40_000 })
    await expect(page.getByText('Skipped: you chose to go on without it')).toBeVisible()
  })

  test('change my answers: opens that step with the answers kept, and the install can be started again', async ({ page, open }) => {
    await open('/install')
    await toSummary(page, 'fail-town')
    await installButton(page).click()
    await expect(page.getByRole('heading', { name: 'The install stopped' })).toBeVisible({ timeout: 40_000 })
    await page.getByRole('alert').getByRole('button', { name: 'Change the database choice' }).click()
    await expect(page.getByRole('heading', { name: 'Database' })).toBeVisible()
    await page.getByRole('button', { name: 'Back to the summary' }).click()
    await expect(page.getByRole('heading', { name: 'Check and install' })).toBeVisible()
    await expect(page.getByText('fail-town', { exact: true })).toBeVisible()
  })

  test('a stopped install is offered again after a reload, with both ways forward', async ({ page, open }) => {
    await open('/install')
    await toSummary(page, 'fail-twice')
    await installButton(page).click()
    await expect(page.getByRole('heading', { name: 'The install stopped' })).toBeVisible({ timeout: 40_000 })
    // The mock keeps its world for the life of the page, so go to the wizard again inside the app.
    await page.getByRole('link', { name: 'Servers' }).click()
    await page.getByRole('link', { name: 'Install new server' }).click()
    await expect(page.getByText(/An install stopped and can carry on/)).toBeVisible()
    await expect(page.getByRole('button', { name: 'Carry on' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Change my answers first' })).toBeVisible()
  })

  test('GitHub is not signed in: the sign-in is offered, and the step is retried by itself', async ({ page, open, scan }) => {
    await open('/install')
    await toSummary(page, 'github-town')
    await installButton(page).click()
    await expect(page.getByRole('heading', { name: 'The install stopped' })).toBeVisible({ timeout: 40_000 })
    await page.getByRole('alert').getByRole('button', { name: 'Sign in to GitHub' }).click()
    await expect(page.getByText(/Kora tries the step again by itself/)).toBeVisible()
    await scan('wizard: GitHub sign-in on a stopped install')
    await page.getByRole('button', { name: 'Sign in to GitHub' }).last().click()
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 50_000 })
  })

  test('"Install the GitHub CLI for me" installs it and carries on by itself', async ({ page, open }) => {
    await open('/install')
    await toSummary(page, 'nogh-town')
    await installButton(page).click()
    await expect(page.getByRole('heading', { name: 'The install stopped' })).toBeVisible({ timeout: 40_000 })
    const alert = page.getByRole('alert').filter({ hasText: 'GitHub CLI' }).first()
    await alert.getByRole('button', { name: 'Install the GitHub CLI for me' }).click()
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 60_000 })
    await expect(page.getByRole('dialog')).toHaveCount(0)
  })

  test('"Start it for me" starts a stopped database and carries on by itself', async ({ page, open }) => {
    await open('/install')
    await toSummary(page, 'nodb-town')
    await installButton(page).click()
    await expect(page.getByRole('heading', { name: 'The install stopped' })).toBeVisible({ timeout: 40_000 })
    await page.getByRole('alert').getByRole('button', { name: 'Start it for me' }).click()
    await expect(page.getByRole('heading', { name: 'All done' })).toBeVisible({ timeout: 60_000 })
  })
})
