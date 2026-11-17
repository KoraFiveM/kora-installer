// The real panel over HTTPS with a real browser: a self-signed certificate from the local CA, plain HTTP redirected, a person invited
// from the command line, a passkey made with Chromium's virtual authenticator (the browser's own WebAuthn code against the real
// server's checks), signing in again with it, and the bare-address case that must fall back to one-time codes.
//
// The browser is started with `--host-resolver-rules` so a NAME (kora-box.test) points at one of this computer's own non-loopback
// addresses: a connection from loopback is "this computer" and would never be offered remote sign-in. Certificate warnings are
// ignored (the CA is not installed in the throw-away browser); the certificate itself is checked by the Rust tests.
import { chromium, expect, test, type Page } from '@playwright/test'
import { execFileSync, spawn, type ChildProcess } from 'node:child_process'
import { createHmac } from 'node:crypto'
import { mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { createServer } from 'node:net'
import { networkInterfaces, tmpdir } from 'node:os'
import { join } from 'node:path'
import { koraBinary } from './panel'

const NAME = 'kora-box.test'

function lanAddress(): string | null {
  for (const list of Object.values(networkInterfaces())) {
    for (const a of list ?? []) if (a.family === 'IPv4' && !a.internal) return a.address
  }
  return null
}

async function freePort(): Promise<number> {
  return new Promise((ok, fail) => {
    const s = createServer()
    s.once('error', fail)
    s.listen(0, '127.0.0.1', () => {
      const { port } = s.address() as { port: number }
      s.close(() => ok(port))
    })
  })
}

function b32decode(s: string): Buffer {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'
  let bits = 0
  let value = 0
  const out: number[] = []
  for (const c of s.replace(/=+$/, '').toUpperCase()) {
    value = (value << 5) | alphabet.indexOf(c)
    bits += 5
    if (bits >= 8) {
      out.push((value >>> (bits - 8)) & 255)
      bits -= 8
    }
  }
  return Buffer.from(out)
}

/** RFC 6238, SHA-1, 6 digits, 30 s steps. */
function totp(secretB32: string, stepOffset = 0): string {
  const step = Math.floor(Date.now() / 30000) + stepOffset
  const msg = Buffer.alloc(8)
  msg.writeBigUInt64BE(BigInt(step))
  const h = createHmac('sha1', b32decode(secretB32)).update(msg).digest()
  const o = h[h.length - 1] & 15
  const n = ((h[o] & 0x7f) << 24) | (h[o + 1] << 16) | (h[o + 2] << 8) | h[o + 3]
  return String(n % 1_000_000).padStart(6, '0')
}

async function virtualAuthenticator(page: Page) {
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('WebAuthn.enable')
  await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: { protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true },
  })
}

test.describe('a real panel over HTTPS', () => {
  test.skip(!lanAddress(), 'this computer has no non-loopback address to reach the panel from')

  test('invite, passkey enrolment, sign-in again, plain HTTP redirect, and one-time codes on a bare address', async () => {
    test.setTimeout(180_000)
    const ip = lanAddress()!
    const root = mkdtempSync(join(tmpdir(), 'kora-e2e-remote-'))
    const home = join(root, 'home')
    const koraHome = join(home, '.kora')
    mkdirSync(koraHome, { recursive: true })
    const env: NodeJS.ProcessEnv = { PATH: process.env.PATH, HOME: home, USERPROFILE: home, KORA_HOME: koraHome, KORA_TAILSCALE: '0', XDG_CONFIG_HOME: join(home, '.config'), XDG_DATA_HOME: join(home, '.local', 'share') }
    const kora = koraBinary()
    const run = (...args: string[]) => execFileSync(kora, args, { env, encoding: 'utf8' })
    run('panel', 'tls', 'self-signed', '--name', NAME, '--name', ip, '--name', '127.0.0.1')
    const link = (who: string) => /#enrol=([0-9a-f]+)/.exec(run('panel', 'user', 'invite', who, '--role', 'admin'))![1]
    const sam = link('sam')
    const amy = link('amy')
    const port = await freePort()
    let out = ''
    const panel: ChildProcess = spawn(kora, ['panel', '--foreground', '--no-open', '--bind', '0.0.0.0', '--allow-public', '--remote', '--port', String(port)], { env, stdio: ['ignore', 'pipe', 'pipe'] })
    panel.stdout?.on('data', (d) => (out += String(d)))
    panel.stderr?.on('data', (d) => (out += String(d)))
    const browser = await chromium.launch({ args: [`--host-resolver-rules=MAP ${NAME} ${ip}`] })
    try {
      for (const t0 = Date.now(); !out.includes('Press Ctrl+C'); ) {
        if (Date.now() - t0 > 60_000) throw new Error(`the panel did not start:\n${out}`)
        await new Promise((r) => setTimeout(r, 150))
      }
      expect(out).toContain(`https://0.0.0.0:${port}`)
      expect(out).not.toMatch(/NO HTTPS certificate/)
      const ctx = await browser.newContext({ ignoreHTTPSErrors: true })
      const page = await ctx.newPage()
      await virtualAuthenticator(page)
      

      // Plain HTTP on the TLS port is sent to https.
      await page.goto(`http://${NAME}:${port}/`)
      await expect(page).toHaveURL(new RegExp(`^https://${NAME}:${port}/`))

      // Enrol a passkey with the one-time link.
      await page.goto(`https://${NAME}:${port}/#enrol=${sam}`)
      await expect(page.getByRole('heading', { name: 'Welcome, sam' })).toBeVisible({ timeout: 30_000 })
      await page.getByLabel('Name this device').fill('Test laptop')
      await page.getByRole('button', { name: 'Set up a passkey' }).click()
      await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible({ timeout: 30_000 })
      // Secure session cookie, and the link is gone from the address bar.
      const cookies = await ctx.cookies()
      expect(cookies.find((c) => c.name.startsWith('kora_session_'))?.secure).toBe(true)
      expect(page.url()).not.toMatch(/enrol=/)

      // The link was single use.
      const other = await ctx.newPage()
      await other.goto(`https://${NAME}:${port}/#enrol=${sam}`)
      await expect(other.getByText(/expired or was already used/)).toBeVisible({ timeout: 30_000 })
      await other.close()

      // Signed-in devices and passkeys are listed; sign out; sign in again with the passkey.
      await page.getByRole('link', { name: 'Settings', exact: true }).click()
      await expect(page.getByRole('heading', { name: 'Your sign-in' })).toBeVisible()
      await expect(page.getByText('Test laptop')).toBeVisible()
      await page.getByRole('button', { name: 'Sign out', exact: true }).click()
      await expect(page.getByRole('heading', { name: 'Sign in to Kora' })).toBeVisible({ timeout: 30_000 })
      await page.getByLabel('Your name').fill('sam')
      await page.getByRole('button', { name: 'Sign in with a passkey' }).click()
      await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible({ timeout: 30_000 })

      // A bare address cannot have a passkey: the page says so and one-time codes work.
      const ctx2 = await browser.newContext({ ignoreHTTPSErrors: true })
      const bare = await ctx2.newPage()
      await virtualAuthenticator(bare)
      await bare.goto(`https://${ip}:${port}/#enrol=${amy}`)
      await expect(bare.getByRole('heading', { name: 'Welcome, amy' })).toBeVisible({ timeout: 30_000 })
      await expect(bare.getByText(/name, not an address|opened by a name/)).toBeVisible()
      await expect(bare.getByRole('button', { name: 'Set up a passkey' })).toHaveCount(0)
      await bare.getByRole('button', { name: 'Set up one-time codes' }).click()
      const secret = (await bare.getByLabel('Setup key').textContent())!.trim()
      await bare.getByLabel('Choose a password').fill('plum otter lantern quietly')
      await bare.getByLabel('Enter the 6-digit code the app shows').fill(totp(secret))
      await bare.getByRole('button', { name: 'Finish set-up' }).click()
      await expect(bare.getByRole('list', { name: 'Recovery codes' }).getByRole('listitem')).toHaveCount(10, { timeout: 30_000 })
      await bare.getByRole('button', { name: 'I have saved them, carry on' }).click()
      await expect(bare.getByRole('navigation', { name: 'Main' })).toBeVisible({ timeout: 30_000 })
      await ctx2.close()
      await ctx.close()
    } finally {
      await browser.close()
      panel.kill('SIGTERM')
      await new Promise((r) => setTimeout(r, 500))
      if (panel.exitCode === null) panel.kill('SIGKILL')
      rmSync(root, { recursive: true, force: true })
    }
  })
})
