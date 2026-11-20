import { test, expect } from '../fixtures'
import type { Page } from '@playwright/test'

// About signed-out and key-less states, not the setup checklist: start with the setup undone.
test.use({ setupState: 'pending' })

// The page of one resource, in the mock layer. The mock lives in the page, so a reload forgets it: "the notice survives a
// reload and shows for other users" is covered by the Rust tests (resourcestate) and the real-panel smoke; here the notice
// is followed across navigation away and back, which also reads it from the (mock) server again.

async function signInOnGithubTab(page: Page) {
  await page.getByRole('tab', { name: 'GitHub', exact: true }).click()
  await page.getByRole('tabpanel').getByRole('button', { name: 'Sign in to GitHub' }).click()
  await expect(page.getByRole('tabpanel').getByText('Signed in', { exact: true })).toBeVisible({ timeout: 30_000 })
}

const header = (page: Page, id: string) => page.getByRole('heading', { name: id, level: 1 }).locator('xpath=..')

test.describe('the page of a resource', () => {
  test('opens from the Resources list, and the back link returns to it', async ({ page, open, scan }) => {
    await open('/servers/kora-live?tab=resources')
    await page.getByRole('link', { name: 'Open the page of kora_core' }).click()
    await expect(page).toHaveURL(/\/servers\/kora-live\/resources\/kora_core$/)
    await expect(page.getByRole('heading', { name: 'kora_core', level: 1 })).toBeVisible()
    await expect(page.getByRole('button', { name: /Check for update/ })).toBeVisible()
    await scan('resource page')
    await page.getByRole('link', { name: /Resources of kora-live/ }).click()
    await expect(page).toHaveURL(/tab=resources/)
    // A deep link works on its own, and an unknown resource says so.
    await open('/servers/kora-live/resources/kora_core')
    await expect(page.getByRole('heading', { name: 'kora_core', level: 1 })).toBeVisible()
  })

  test('check for update and update this resource; the notice waits for a start of the server', async ({ page, open }) => {
    await open('/servers/kora-dev/resources/kora_core')
    await page.getByRole('button', { name: /Check for update/ }).click()
    await expect(page.getByText('Ready to update').first()).toBeVisible({ timeout: 30_000 })
    await page.getByRole('button', { name: 'Start the server' }).click()
    await expect(page.getByText('Running', { exact: true }).first()).toBeVisible({ timeout: 30_000 })
    await page.getByRole('button', { name: /^Update$/ }).click()
    await expect(page.getByText(/Updated\. The running server still has the old version/)).toBeVisible({ timeout: 40_000 })
    const notice = page.getByRole('alert').filter({ hasText: 'Restart needed: the running server still has the old version.' })
    await expect(notice).toContainText('What changed: updated')
    // The live console is off on this server, so only the whole-server restart works, and the page says why.
    await expect(notice.getByRole('button', { name: /Restart this resource/ })).toBeDisabled()
    await notice.getByRole('button', { name: 'Restart the whole server' }).click()
    await expect(page.getByText(/Restart needed/)).toHaveCount(0, { timeout: 30_000 })
  })

  test('a change of the files says "Restart needed" until the resource is restarted (asks first)', async ({ page, open, scan }) => {
    await open('/servers/kora-live/resources/kora_core')
    await expect(page.getByText(/Restart needed/)).toHaveCount(0)
    await page.getByLabel('Branch', { exact: true }).selectOption({ index: 1 })
    // (Released versions: leaving the release needs the tick.)
    await page.getByLabel(/I understand: this resource will run unreleased code/).first().check()
    await page.getByRole('button', { name: 'Switch', exact: true }).click()
    const notice = page.getByRole('alert').filter({ hasText: 'Restart needed: the running server still has the old version.' })
    await expect(notice).toBeVisible()
    await scan('resource page: restart needed')
    // Away and back: the notice is read from the server again, not from the browser.
    await page.getByRole('link', { name: /Resources of kora-live/ }).click()
    await page.getByRole('link', { name: 'Open the page of kora_core' }).click()
    await expect(notice).toBeVisible()
    await notice.getByRole('button', { name: /Restart this resource/ }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Restart resource' }).click()
    await expect(page.getByText(/Restart needed/)).toHaveCount(0)
  })

  test('start, stop and restart on the running server; stop and restart ask first', async ({ page, open }) => {
    await open('/servers/kora-live/resources/kora_core')
    await page.getByRole('button', { name: 'Stop', exact: true }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Stop resource' }).click()
    await expect(page.getByText(/Asked the server to stop kora_core/)).toBeVisible()
    await page.getByRole('button', { name: 'Start', exact: true }).click()
    await expect(page.getByText(/Asked the server to start kora_core/)).toBeVisible()
    await page.getByRole('button', { name: 'Restart', exact: true }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Restart resource' }).click()
    await expect(page.getByText(/Asked the server to restart kora_core/)).toBeVisible()
  })

  test('on a stopped server the buttons say why they are off and offer to start the server', async ({ page, open }) => {
    await open('/servers/kora-dev/resources/kora_core')
    await expect(page.getByRole('button', { name: 'Start', exact: true })).toBeDisabled()
    await expect(page.getByText(/The server is not running\. Start the server first/).first()).toBeVisible()
    await expect(page.getByRole('button', { name: 'Start the server' })).toBeEnabled()
  })

  test('a resource that is not on its normal version is badged on every page that shows it', async ({ page, open, scan }) => {
    await open('/servers/kora-dev/resources/kora_phone')
    // Its own page.
    await expect(header(page, 'kora_phone').getByText('On branch fix-contacts')).toBeVisible()
    // The Resources list.
    await page.getByRole('link', { name: /Resources of kora-dev/ }).click()
    await expect(page.getByRole('link', { name: 'Open the page of kora_phone' }).locator('xpath=ancestor::li[1]').getByText('On branch fix-contacts')).toBeVisible()
    // The update check says it is left alone, with the reason.
    await page.getByRole('button', { name: 'Check for updates' }).click()
    await expect(page.getByText(/Not updated: on another branch/).first()).toBeVisible({ timeout: 30_000 })
    // The snapshots remember which branch it was on.
    await expect(page.getByText('kora_phone was on branch fix-contacts')).toBeVisible()
    // The GitHub tab.
    await page.getByRole('tab', { name: 'GitHub', exact: true }).click()
    await expect(page.getByRole('list', { name: 'Resources and their branches' }).getByText('On branch fix-contacts')).toBeVisible()
    // The Overview and the servers list say how many.
    await page.getByRole('tab', { name: 'Overview', exact: true }).click()
    await expect(page.getByRole('tabpanel').getByText('1 resource not on its normal version').first()).toBeVisible()
    await scan('overview with a resource off its normal version')
    await page.getByRole('link', { name: 'All servers' }).click()
    await expect(page.getByRole('row', { name: /kora-dev/ }).getByText('1 resource not on its normal version')).toBeVisible()
  })

  test('trying a pull request on a server that follows released versions needs a tick, then get the latest and stop trying', async ({ page, open, scan }) => {
    await open('/servers/kora-live')
    await signInOnGithubTab(page)
    await page.getByRole('tab', { name: 'Resources', exact: true }).click()
    await page.getByRole('link', { name: 'Open the page of kora_core' }).click()
    const panel = page.getByRole('heading', { name: 'Pull requests' }).locator('xpath=ancestor::div[contains(@class,"rounded")][1]')
    await expect(panel.getByText('This server follows released versions')).toBeVisible()
    const tryIt = panel.getByRole('button', { name: 'Try it: pull request #41' })
    await expect(tryIt).toBeDisabled()
    await scan('resource page: pull requests on released versions')
    await panel.getByLabel(/I understand: this resource will run unreleased code/).check()
    await tryIt.click()
    await expect(page.getByText(/Now on pull request #41/)).toBeVisible()
    await expect(header(page, 'kora_core').getByText('Trying pull request #41: Fix the payroll rounding')).toBeVisible()
    await expect(page.getByRole('alert').filter({ hasText: 'Restart needed' })).toBeVisible()
    // How far behind it is, then get the latest commits.
    await expect(page.getByTestId('trying-status')).toContainText('GitHub has 2 newer commits')
    await page.getByRole('button', { name: 'Get the latest commits' }).click()
    await expect(page.getByText(/Already trying pull request #41|Now trying pull request #41|Now on pull request #41/).first()).toBeVisible()
    // Stop trying it: back to exactly the released version.
    await page.getByRole('button', { name: 'Stop trying it' }).click()
    await expect(header(page, 'kora_core').getByText(/Trying pull request/)).toHaveCount(0)
  })

  test('details of a pull request, and merging with and without the right to', async ({ page, open }) => {
    await open('/servers/kora-dev')
    await signInOnGithubTab(page)
    await page.getByRole('link', { name: 'Open the page of kora_core' }).click()
    const list = page.getByRole('list', { name: 'Open pull requests' })
    // #41: the account may not merge here, and the page says whose account that is.
    await list.getByText(/payroll rounding/).locator('xpath=ancestor::li[1]').getByRole('button', { name: 'Details' }).click()
    const d41 = page.getByLabel('Pull request #41')
    await expect(d41.getByText(/tests: failing/)).toBeVisible()
    await expect(d41.getByRole('button', { name: /^Merge/ })).toBeDisabled()
    await expect(d41.getByText(/does not have it/)).toBeVisible()
    await expect(d41.getByTestId('acting-as')).toContainText('sam-rp')
    // #3: allowed. Comment, then approve is refused for one's own, then merge after the question.
    await list.getByText(/My experiment/).locator('xpath=ancestor::li[1]').getByRole('button', { name: 'Details' }).click()
    const d3 = page.getByLabel('Pull request #3')
    await d3.getByLabel('Your comment or review').fill('Tested, works')
    await d3.getByRole('button', { name: 'Comment' }).click()
    await expect(d3.getByText('Your comment was added.')).toBeVisible()
    await d3.getByRole('button', { name: 'Approve' }).click()
    await expect(d3.getByText(/does not let an account approve its own pull request/)).toBeVisible()
    await d3.getByRole('button', { name: /^Merge/ }).click()
    const sure = page.getByRole('dialog')
    await expect(sure).toContainText('for real, as sam-rp')
    await sure.getByRole('button', { name: 'Merge it' }).click()
    await expect(d3.getByText(/Pull request #3 was merged/)).toBeVisible()
  })

  test('your changes: the diff, commit, push as the signed-in account, no rights, and opening a pull request', async ({ page, open, scan }) => {
    await open('/servers/kora-dev')
    await signInOnGithubTab(page)
    // kora_jobs has changes of its own.
    await page.getByRole('link', { name: 'Open the page of kora_jobs' }).click()
    const files = page.getByRole('list', { name: 'Changed files' })
    await expect(files.getByText('config.lua')).toBeVisible()
    await page.getByRole('button', { name: 'Show the changes line by line' }).click()
    await expect(page.getByLabel('Changes, line by line')).toContainText('+Config.Pay = 120')
    await scan('resource page: your changes')
    await page.getByLabel(/Save these changes with a message/).fill('Raise the pay')
    await page.getByRole('button', { name: /Save the changes/ }).click()
    await expect(page.getByText(/Saved 3 files as commit/)).toBeVisible()
    await page.getByRole('button', { name: /Push to sam-rp\/kora_jobs/ }).click()
    await expect(page.getByText(/Pushed/).first()).toBeVisible()
    await expect(page.getByTestId('acting-as').first()).toContainText('sam-rp')
    // Open a pull request from it.
    await page.getByRole('button', { name: 'Check my changes' }).click()
    await expect(page.getByText('Ready to send')).toBeVisible()
    await page.getByLabel('Title').first().fill('Raise the pay')
    await page.getByRole('button', { name: 'Open pull request' }).click()
    await expect(page.getByText('Pull request opened')).toBeVisible()
  })

  test('pushing without the right to explains what to do; a plain copy has nowhere to push', async ({ page, open }) => {
    await open('/servers/kora-dev')
    await signInOnGithubTab(page)
    await page.getByRole('link', { name: 'Open the page of kora_phone' }).click()
    await page.getByRole('button', { name: /Push to sam-rp\/kora_phone/ }).click()
    await expect(page.getByText(/not allowed to push to sam-rp\/kora_phone/)).toBeVisible()
    await open('/servers/kora-live/resources/kora_core')
    await expect(page.getByText(/plain copy of the original, so there is nowhere of yours/)).toBeVisible()
  })

  test('make my own copy: signed out is guided, then it turns the clone into your own copy', async ({ page, open, scan }) => {
    await open('/servers/kora-live/resources/kora_core')
    await page.getByRole('button', { name: 'Make my own copy (fork)' }).click()
    await expect(page.getByText('You are not signed in to GitHub.').first()).toBeVisible()
    await page.getByRole('link', { name: /Resources of kora-live/ }).click()
    await signInOnGithubTab(page)
    await page.getByRole('tab', { name: 'Resources', exact: true }).click()
    await page.getByRole('link', { name: 'Open the page of kora_core' }).click()
    await scan('resource page: make my own copy')
    await page.getByRole('button', { name: 'Make my own copy (fork)' }).click()
    await expect(page.getByText(/kora_core now has your own copy \(sam-rp\/kora_core\)/)).toBeVisible()
  })

  test('issues and releases', async ({ page, open }) => {
    await open('/servers/kora-live')
    await signInOnGithubTab(page)
    await page.getByRole('tab', { name: 'Resources', exact: true }).click()
    await page.getByRole('link', { name: 'Open the page of kora_core' }).click()
    await expect(page.getByText(/The phone app crashes/)).toBeVisible()
    await expect(page.getByText(/This server is pinned to/)).toContainText('1 newer release exists')
    await expect(page.getByRole('list', { name: 'Releases' }).getByText('Newer')).toBeVisible()
  })

  test('"Which versions" can be changed later, and nothing moves', async ({ page, open }) => {
    await open('/servers/kora-live?tab=resources')
    await expect(page.getByRole('heading', { name: 'Which versions' })).toBeVisible()
    await page.getByLabel(/Latest changes \(the main branch\)/).check()
    await page.getByRole('button', { name: 'Save', exact: true }).click()
    await expect(page.getByText(/nothing has changed yet/)).toBeVisible()
  })
})
