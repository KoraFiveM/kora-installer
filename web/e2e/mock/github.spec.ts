import { test, expect } from '../fixtures'
import type { Page } from '@playwright/test'

const row = (page: Page, id: string) => page.getByRole('list', { name: 'Resources and their branches', includeHidden: true }).getByText(id, { exact: true }).locator('xpath=ancestor::li[1]')

async function openGithubTab(page: Page) {
  await page.getByRole('tab', { name: 'GitHub', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Change the branch of kora_core' })).toBeVisible()
}

async function signIn(page: Page) {
  await page.getByRole('tabpanel').getByRole('button', { name: 'Sign in to GitHub' }).click()
  await expect(page.getByRole('tabpanel').getByText('Signed in', { exact: true })).toBeVisible({ timeout: 30_000 })
}

test.describe('GitHub tab: branches and pull requests', () => {
  test('shows where every resource is, and says updates leave a switched one alone', async ({ page, open, scan }) => {
    await open('/servers/kora-dev')
    await openGithubTab(page)
    await expect(row(page, 'kora_phone')).toContainText('Branch fix-contacts')
    await expect(row(page, 'kora_jobs')).toContainText('2 changed files')
    await expect(page.getByText(/1 resource is not on the normal version/)).toBeVisible()
    await expect(page.getByText(/Updates leave it alone/)).toBeVisible()
    await scan('GitHub tab: branches')
  })

  test('switches a clean resource to a branch of the fork, and back to the normal version', async ({ page, open, scan }) => {
    await open('/servers/kora-dev')
    await openGithubTab(page)
    await page.getByRole('button', { name: 'Change the branch of kora_core' }).click()
    const dialog = page.getByRole('dialog', { name: /Branch of kora_core/ })
    const pick = dialog.getByLabel('Branch', { exact: true })
    await expect(pick.getByRole('option', { name: 'feature/new-ui' })).toBeAttached()
    await pick.selectOption('origin:feature/new-ui')
    await scan('GitHub tab: branch dialog')
    await dialog.getByRole('button', { name: 'Switch' }).click()
    await expect(dialog.getByText(/Now on branch feature\/new-ui/)).toBeVisible()
    await expect(dialog.getByText(/The server is not running, so it will use the new version when it starts/)).toBeVisible()
    await expect(row(page, 'kora_core')).toContainText('Branch feature/new-ui')
    await dialog.getByRole('button', { name: 'Back to the normal version' }).click()
    await expect(dialog.getByText(/Now on the normal version \(dev\)/)).toBeVisible()
  })

  test('changes of your own are never lost: keep them aside, then bring them back', async ({ page, open, scan }) => {
    await open('/servers/kora-dev')
    await openGithubTab(page)
    await page.getByRole('button', { name: 'Change the branch of kora_jobs' }).click()
    const dialog = page.getByRole('dialog', { name: /Branch of kora_jobs/ })
    const pick = dialog.getByLabel('Branch', { exact: true })
    await expect(pick.getByRole('option', { name: 'feature/new-ui' })).toBeAttached()
    await pick.selectOption('origin:feature/new-ui')
    await dialog.getByRole('button', { name: 'Switch' }).click()
    const ask = page.getByRole('dialog', { name: 'You have changes that are not saved' })
    await expect(ask.getByText('config.lua')).toBeVisible()
    await expect(ask.getByText('client/main.lua')).toBeVisible()
    await scan('GitHub tab: changes not saved')
    await ask.getByRole('button', { name: 'Keep my changes aside' }).click()
    await expect(dialog.getByText(/Your changes were kept aside/)).toBeVisible()
    await dialog.getByRole('button', { name: /Bring my changes back: kept-aside set 1/ }).click()
    await expect(dialog.getByText(/Your changes are back/)).toBeVisible()
  })

  test('throwing changes away is asked twice, with a warning; going back or cancelling does nothing', async ({ page, open }) => {
    await open('/servers/kora-dev')
    await openGithubTab(page)
    await page.getByRole('button', { name: 'Change the branch of kora_jobs' }).click()
    const dialog = page.getByRole('dialog', { name: /Branch of kora_jobs/ })
    const pick = dialog.getByLabel('Branch', { exact: true })
    await expect(pick.getByRole('option', { name: 'feature/new-ui' })).toBeAttached()
    await pick.selectOption('origin:feature/new-ui')
    await dialog.getByRole('button', { name: 'Switch' }).click()
    const ask = page.getByRole('dialog', { name: 'You have changes that are not saved' })
    await ask.getByRole('button', { name: 'Throw my changes away…' }).click()
    // The second question has its own title, so look at the top dialog rather than by the first one's name.
    const sure = page.getByRole('dialog').last()
    await expect(sure.getByText(/This cannot be undone/)).toBeVisible()
    await sure.getByRole('button', { name: 'Go back' }).click()
    await expect(row(page, 'kora_jobs')).toContainText('2 changed files')
    // Cancel: leaves the dialog without doing anything.
    await page.getByRole('dialog').last().getByRole('button', { name: 'Cancel, change nothing' }).click()
    await expect(page.getByRole('dialog', { name: 'You have changes that are not saved' })).toBeHidden()
    await expect(row(page, 'kora_jobs')).toContainText('2 changed files')
    // Asked again, and this time thrown away.
    await dialog.getByRole('button', { name: 'Switch' }).click()
    await ask.getByRole('button', { name: 'Throw my changes away…' }).click()
    await page.getByRole('dialog').last().getByRole('button', { name: 'Throw my changes away', exact: true }).click()
    await expect(dialog.getByText(/Now on branch feature\/new-ui/)).toBeVisible()
    await expect(row(page, 'kora_jobs')).not.toContainText('2 changed files')
  })

  test('pull requests: someone else\'s code is warned about before anyone tries one', async ({ page, open, scan }) => {
    await open('/servers/kora-dev')
    await openGithubTab(page)
    await signIn(page)
    await page.getByRole('button', { name: 'Change the branch of kora_core' }).click()
    const dialog = page.getByRole('dialog', { name: /Branch of kora_core/ })
    await expect(dialog.getByRole('note')).toContainText(/someone else's code.*runs on your server.*people you trust/)
    await dialog.getByRole('button', { name: 'Show open pull requests' }).click()
    const prs = dialog.getByRole('list', { name: 'Open pull requests' })
    await expect(prs.getByText('Original repository')).toHaveCount(2)
    await expect(prs.getByText('Your fork')).toBeVisible()
    await expect(prs.getByText('Draft')).toBeVisible()
    await scan('GitHub tab: pull requests')
    await prs.getByRole('button', { name: 'Try it: pull request #41' }).click()
    await expect(dialog.getByText(/Now on pull request #41/)).toBeVisible()
    await expect(row(page, 'kora_core')).toContainText('Trying pull request #41')
  })

  test('pull request numbers that cannot be tried are explained without leaving the dialog', async ({ page, open }) => {
    await open('/servers/kora-dev')
    await openGithubTab(page)
    await signIn(page)
    await page.getByRole('button', { name: 'Change the branch of kora_core' }).click()
    const dialog = page.getByRole('dialog', { name: /Branch of kora_core/ })
    const box = dialog.getByLabel('Pull request number or address')
    const tryIt = dialog.getByRole('button', { name: /^Try it$/ })
    for (const [text, answer] of [['12', /already been merged/], ['999', /There is no pull request #999/], ['rm -rf /', /doesn't look like a pull request/], ['https://github.com/someone/else/pull/3', /not this resource's repository or your fork of it/]] as const) {
      await box.fill(text)
      await tryIt.click()
      await expect(dialog.getByText(answer)).toBeVisible()
    }
  })

  test('without being signed in, a pull request list says so plainly', async ({ page, open }) => {
    await open('/servers/kora-dev')
    await openGithubTab(page)
    await page.getByRole('button', { name: 'Change the branch of kora_core' }).click()
    const dialog = page.getByRole('dialog', { name: /Branch of kora_core/ })
    await dialog.getByRole('button', { name: 'Show open pull requests' }).click()
    await expect(dialog.getByText('You are not signed in to GitHub.')).toBeVisible()
  })

  test('many at once: shows who has the branch, switches only those, and asks about changes', async ({ page, open }) => {
    await open('/servers/kora-dev')
    await openGithubTab(page)
    await page.getByLabel('Branch name').fill('feature/new-ui')
    await page.getByRole('button', { name: 'Check which resources have it' }).click()
    await expect(page.getByRole('list', { name: 'Which resources have the branch' })).toBeVisible()
    await expect(page.getByText('3 of 4 resources have feature/new-ui')).toBeVisible()
    await page.getByRole('button', { name: /Switch the 3 that have it/ }).click()
    const done = page.getByRole('list', { name: 'What happened to each resource' })
    await expect(done.getByText('Has changes of yours')).toBeVisible()
    await page.getByRole('button', { name: /Choose what to do with the 1 that has changes/ }).click()
    await page.getByRole('dialog', { name: 'You have changes that are not saved' }).getByRole('button', { name: 'Keep my changes aside' }).click()
    await expect(page.getByText(/Your changes are kept aside/)).toBeVisible()
  })

  test('after switching a resource, the Updates check leaves it alone', async ({ page, open }) => {
    await open('/servers/kora-dev')
    await openGithubTab(page)
    await page.getByRole('button', { name: 'Change the branch of kora_core' }).click()
    const dialog = page.getByRole('dialog', { name: /Branch of kora_core/ })
    await dialog.getByLabel('Branch', { exact: true }).selectOption('origin:feature/new-ui')
    await dialog.getByRole('button', { name: 'Switch' }).click()
    await expect(dialog.getByText(/Now on branch feature\/new-ui/)).toBeVisible()
    await dialog.getByRole('button', { name: 'Close' }).first().click()
    await page.getByRole('tab', { name: 'Resources', exact: true }).click()
    await page.getByRole('button', { name: 'Check for updates' }).click()
    await expect(page.getByRole('checkbox', { name: 'Update kora_core' })).toBeDisabled({ timeout: 30_000 })
    await expect(page.getByText(/Not updated: on another branch/).first()).toBeVisible()
  })

  test('report an issue and open a pull request need the sign-in first, then appear', async ({ page, open, scan }) => {
    await open('/servers/kora-dev')
    await openGithubTab(page)
    await expect(page.getByRole('heading', { name: 'Report an issue' })).toHaveCount(0)
    await signIn(page)
    await expect(page.getByRole('heading', { name: 'Report an issue' })).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Open a pull request' })).toBeVisible()
    await scan('GitHub tab: signed in')
  })
})
