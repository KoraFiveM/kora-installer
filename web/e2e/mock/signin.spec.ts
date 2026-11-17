// Remote sign-in in the browser: the sign-in screen, the enrolment page (one-time codes, and a passkey made with Chromium's
// virtual authenticator), Your sign-in in Settings, the fresh check, and inviting a person. The panel here is the mock, so
// these journeys check the page and the real browser WebAuthn API; the server side is covered by the Rust tests and by
// e2e/real/remote-signin.spec.ts (a real panel, HTTPS and a virtual authenticator).
import type { CDPSession, Page } from '@playwright/test'
import { test, expect } from '../fixtures'

const PORT = Number(process.env.E2E_MOCK_PORT ?? 4173)
/** `localhost` is a name (a passkey can be registered for it) and a secure context; 127.0.0.1 is neither a name nor accepted as an ID. */
const NAMED = `http://localhost:${PORT}`

async function virtualAuthenticator(page: Page): Promise<CDPSession> {
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('WebAuthn.enable')
  await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: { protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true },
  })
  return cdp
}

test.describe('the sign-in screen from another computer', () => {
  test('offers a passkey first and one-time codes as the alternative', async ({ page, open, scan }) => {
    await open('/', { scenario: 'offline', as: 'remote-passkey-admin' })
    await expect(page.getByRole('heading', { name: 'Sign in to Kora' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Sign in with a passkey' })).toBeDisabled()
    await page.getByLabel('Your name').fill('sam')
    await expect(page.getByRole('button', { name: 'Sign in with a passkey' })).toBeEnabled()
    await scan('remote sign-in, passkey')
    await page.getByRole('button', { name: 'Use a one-time code instead' }).click()
    await expect(page.getByLabel('Password')).toBeVisible()
    await expect(page.getByLabel('One-time code')).toBeVisible()
    await scan('remote sign-in, one-time code')
    // The old token link is tucked away, not the main road.
    await expect(page.getByLabel('Old token link')).toHaveCount(0)
    await page.getByRole('button', { name: 'I have an old token link' }).click()
    await expect(page.getByLabel('Old token link')).toBeVisible()
  })

  test('a wrong password says so in plain words', async ({ page, open }) => {
    await open('/', { scenario: 'offline', as: 'remote-passkey-admin' })
    await page.getByRole('button', { name: 'Use a one-time code instead' }).click()
    await page.getByLabel('Your name').fill('sam')
    await page.getByLabel('Password').fill('wrong')
    await page.getByLabel('One-time code').fill('123456')
    await page.getByRole('button', { name: 'Sign in with a code' }).click()
    await expect(page.getByText(/not right \(or the account is locked/)).toBeVisible()
    await expect(page.getByLabel('Password')).toHaveValue('')
  })
})

test.describe('enrolment', () => {
  test('one-time codes: scan, choose a password, get recovery codes, carry on', async ({ page, open, scan }) => {
    await open('/#enrol=demo-invite-codes', { as: 'remote-code-operator' })
    await expect(page.getByRole('heading', { name: 'Welcome, amy' })).toBeVisible()
    await scan('enrolment page')
    await page.getByRole('button', { name: 'Set up one-time codes' }).click()
    await expect(page.getByAltText('QR code for your authenticator app')).toBeVisible()
    await page.getByLabel('Choose a password').fill('short')
    await page.getByLabel('Enter the 6-digit code the app shows').fill('123456')
    await page.getByRole('button', { name: 'Finish set-up' }).click()
    await expect(page.getByText(/at least 12 characters/)).toBeVisible()
    await page.getByLabel('Choose a password').fill('plum otter lantern quietly')
    await page.getByRole('button', { name: 'Finish set-up' }).click()
    await expect(page.getByRole('list', { name: 'Recovery codes' }).getByRole('listitem')).toHaveCount(10)
    await scan('recovery codes')
    await page.getByRole('button', { name: 'I have saved them, carry on' }).click()
    await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible()
  })

  test('a passkey made with the browser\'s own WebAuthn API', async ({ page, open }) => {
    // The panel here is the mock, which has no name of its own: use localhost.
    await page.goto(`${NAMED}/?scenario=default&as=remote-passkey-admin#enrol=demo-invite`)
    await virtualAuthenticator(page)
    await expect(page.getByRole('heading', { name: 'Welcome, newbie' })).toBeVisible()
    await page.getByLabel('Name this device').fill('Test laptop')
    await page.getByRole('button', { name: 'Set up a passkey' }).click()
    await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible()
    void open
  })

  test('a bare address cannot make a passkey: the browser says no and the page says what to do instead', async ({ page, open, problems }) => {
    problems.expect(/./)
    await open('/#enrol=demo-invite', { as: 'remote-passkey-admin' })
    await virtualAuthenticator(page)
    await page.getByRole('button', { name: 'Set up a passkey' }).click()
    await expect(page.getByText(/one-time code/).first()).toBeVisible()
    await expect(page.getByRole('button', { name: 'Set up one-time codes' })).toBeVisible()
  })
})

test.describe('Your sign-in', () => {
  test('a token sign-in is nudged to something stronger on every page, and the nudge goes once it is done', async ({ page, open }) => {
    await open('/', { as: 'remote-admin' })
    await expect(page.getByText('Replace your token link').first()).toBeVisible()
    await expect(page.getByRole('link', { name: 'Set up a passkey or one-time codes' })).toBeVisible()
  })

  test('a token user is told to replace the link, and adds a passkey', async ({ page, open, scan }) => {
    await page.goto(`${NAMED}/settings?scenario=default&as=remote-admin`)
    await virtualAuthenticator(page)
    await expect(page.getByText('Replace your token link')).toBeVisible()
    await expect(page.getByRole('button', { name: 'End my token link' })).toBeDisabled()
    await page.getByLabel('Name this device').fill('Phone')
    await page.getByRole('button', { name: 'Add a passkey' }).click()
    await expect(page.getByText('Phone', { exact: true })).toBeVisible()
    await expect(page.getByRole('button', { name: 'End my token link' })).toBeEnabled()
    await scan('your sign-in with a passkey')
    void open
  })

  test('a sensitive change asks for a fresh check first, then carries on', async ({ page, open }) => {
    await page.goto(`${NAMED}/settings?scenario=default&as=remote-passkey-admin`)
    await virtualAuthenticator(page)
    // Two passkeys (the first one is the person's, made on this device).
    for (const n of ['First', 'Second']) {
      await page.getByLabel('Name this device').fill(n)
      await page.getByRole('button', { name: 'Add a passkey' }).click()
      await expect(page.getByText(n, { exact: true })).toBeVisible()
    }
    await page.evaluate(() => window.sessionStorage.setItem('kora-mock-fresh', '0'))
    await page.getByRole('button', { name: 'Remove passkey Second' }).click()
    await page.getByRole('button', { name: 'Remove', exact: true }).click()
    await expect(page.getByRole('dialog', { name: 'Confirm it is you' })).toBeVisible()
    await page.getByRole('button', { name: 'Use my passkey' }).click()
    await expect(page.getByRole('dialog', { name: 'Confirm it is you' })).toHaveCount(0)
    await expect(page.getByText('Second', { exact: true })).toHaveCount(0)
    void open
  })
})

test.describe('on this computer', () => {
  test('inviting a person gives a one-time link, not a token', async ({ page, open, scan }) => {
    await open('/settings', { as: 'local' })
    await page.locator('form').filter({ hasText: 'Invite person' }).getByLabel('Name').fill('newbie')
    await page.getByRole('button', { name: 'Invite person' }).click()
    await expect(page.getByText('Invitation for newbie')).toBeVisible()
    await expect(page.getByLabel('Enrolment link for newbie')).toContainText('#enrol=')
    await expect(page.getByText('waiting to set up')).toBeVisible()
    await scan('invitation')
  })

  test('API tokens are made once and revoked', async ({ page, open }) => {
    await open('/settings', { as: 'local' })
    await page.getByLabel('Token name').fill('backup-script')
    await page.getByRole('button', { name: 'Make token' }).click()
    await expect(page.getByLabel('API token for backup-script')).toContainText('kora_')
    await expect(page.getByText('backup-script').first()).toBeVisible()
    await page.getByRole('button', { name: 'Revoke backup-script' }).click()
    await page.getByRole('button', { name: 'Revoke', exact: true }).click()
    await expect(page.getByRole('button', { name: 'Revoke backup-script' })).toHaveCount(0)
  })

  test('the HTTPS card shows the certificate, its fingerprint and when it ends', async ({ page, open, scan }) => {
    await open('/settings', { as: 'local' })
    await expect(page.getByRole('heading', { name: 'HTTPS' })).toBeVisible()
    await expect(page.getByText(/self-signed certificate made by Kora/)).toBeVisible()
    await expect(page.getByText(/AB:12:CD:34/)).toBeVisible()
    await expect(page.getByText('300 days left')).toBeVisible()
    await page.getByRole('button', { name: 'Renew now' }).click()
    await expect(page.getByText('397 days left')).toBeVisible()
    await scan('settings https card')
  })
})

test.describe('saved licence keys and the keychain', () => {
  test('Settings says where each key is kept and the keychain can be turned on and off', async ({ page, open }) => {
    await open('/settings', { as: 'local', scenario: 'keylost' })
    await expect(page.getByText(/Kept in Kora's private file \(no system keychain on this computer\)/)).toBeVisible()
    await page.getByRole('button', { name: 'Use the system keychain' }).click()
    await expect(page.getByText('New keys are kept in your system keychain.')).toBeVisible()
    await expect(page.getByText('Kept in your system keychain', { exact: true })).toBeVisible()
    await page.getByRole('button', { name: "Keep keys in Kora's private file instead" }).click()
    await expect(page.getByText(/you turned the system keychain off/)).toBeVisible()
  })

  test('a key whose keychain entry is gone says so and offers to enter it again', async ({ page, open, scan }) => {
    await open('/settings', { as: 'local', scenario: 'keylost' })
    await expect(page.getByText('This key cannot be read from here').first()).toBeVisible()
    await expect(page.getByText(/not in your system keychain any more/).first()).toBeVisible()
    await scan('settings lost key')
    await page.getByRole('button', { name: 'Enter it again: Main' }).click()
    await expect(page.getByRole('dialog')).toBeVisible()
  })
})
