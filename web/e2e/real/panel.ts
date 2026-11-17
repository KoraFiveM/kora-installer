// Fixtures for the smoke layer against the REAL panel binary. Nothing here installs a server.
//
// Isolation (the same idea as scripts/smoke-linux/common.sh): every run gets a throw-away HOME and KORA_HOME inside a
// new temporary folder and a spare port. The panel runs in the foreground (`kora panel --foreground --no-open`);
// the tests sign in with a one-time code link asked for like `kora panel --print-link` does. It is stopped when
// the test run ends.
import { test as base, expect, type Page } from '@playwright/test'
import { spawn, type ChildProcess } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const EXE = process.platform === 'win32' ? 'kora.exe' : 'kora'

/** The binary under test: KORA_BIN, else the debug or release build of this checkout. */
export function koraBinary(): string {
  const candidates = [
    process.env.KORA_BIN,
    process.env.CARGO_TARGET_DIR && join(process.env.CARGO_TARGET_DIR, 'debug', EXE),
    process.env.CARGO_TARGET_DIR && join(process.env.CARGO_TARGET_DIR, 'release', EXE),
    resolve(HERE, '../../../target/debug', EXE),
    resolve(HERE, '../../../target/release', EXE),
  ].filter((p): p is string => !!p)
  const found = candidates.find((p) => existsSync(p))
  if (!found) throw new Error(`no kora binary found: build one (cargo build -p kora-cli) or set KORA_BIN. Looked in:\n  ${candidates.join('\n  ')}`)
  return found
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

export interface Panel {
  /** `http://127.0.0.1:PORT` */
  origin: string
  /** A fresh one-time sign-in link (`/#code=...`), asked for the way `kora panel --print-link` does. It works once. */
  newLink: () => Promise<string>
  /** The throw-away folder holding HOME, KORA_HOME and the fake servers. */
  root: string
  /** Everything the panel printed (for failure reports). */
  log: () => string
  /** A folder that looks like a FiveM server to Kora (server.cfg and a txData folder), under `root`. */
  fakeServer: (name: string, gamePort?: number) => string
}

async function startPanel(): Promise<{ panel: Panel; stop: () => Promise<void> }> {
  const root = mkdtempSync(join(tmpdir(), 'kora-e2e-'))
  const home = join(root, 'home')
  const koraHome = join(home, '.kora')
  mkdirSync(koraHome, { recursive: true })
  const port = await freePort()
  const env: NodeJS.ProcessEnv = {
    PATH: process.env.PATH,
    HOME: home,
    USERPROFILE: home,
    KORA_HOME: koraHome,
    // No desktop, no browser, no Tailscale questions: this is a plain local panel.
    KORA_TAILSCALE: '0',
    XDG_CONFIG_HOME: join(home, '.config'),
    XDG_DATA_HOME: join(home, '.local', 'share'),
    XDG_STATE_HOME: join(home, '.local', 'state'),
    XDG_RUNTIME_DIR: process.env.XDG_RUNTIME_DIR,
    SystemRoot: process.env.SystemRoot,
  }
  let out = ''
  const child: ChildProcess = spawn(koraBinary(), ['panel', '--foreground', '--no-open', '--port', String(port)], { env, stdio: ['ignore', 'pipe', 'pipe'] })
  child.stdout?.on('data', (d) => (out += String(d)))
  child.stderr?.on('data', (d) => (out += String(d)))
  let exited: number | null = null
  child.on('exit', (c) => (exited = c ?? -1))

  // Wait until the panel is up: it announces its address, and records its sign-in token in KORA_HOME.
  const running = join(koraHome, 'panel-running.json')
  const deadline = Date.now() + 60_000
  while (!(out.includes(`http://127.0.0.1:${port}`) && existsSync(running))) {
    if (exited !== null) throw new Error(`kora panel stopped (exit ${exited}) before it was ready. Output:\n${out}`)
    if (Date.now() > deadline) throw new Error(`kora panel was not ready within 60 s. Output:\n${out}`)
    await new Promise((r) => setTimeout(r, 150))
  }
  const origin = `http://127.0.0.1:${port}`
  const newLink = async () => {
    // What `kora panel --print-link` does: prove who we are with the token, get a code that works once.
    const { token } = JSON.parse(readFileSync(running, 'utf8')) as { token: string }
    const r = await fetch(`${origin}/api/v1/session/link`, { method: 'POST', headers: { Authorization: `Bearer ${token}`, 'X-Kora': '1', 'Content-Type': 'application/json' }, body: '{}' })
    if (!r.ok) throw new Error(`the panel refused to make a sign-in link: HTTP ${r.status}`)
    const { code } = (await r.json()) as { code: string }
    return `${origin}/#code=${code}`
  }

  const panel: Panel = {
    origin,
    newLink,
    root,
    log: () => out,
    fakeServer: (name, gamePort = 30199) => {
      const dir = join(root, 'servers', name)
      mkdirSync(join(dir, 'txData'), { recursive: true })
      mkdirSync(join(dir, 'resources'), { recursive: true })
      writeFileSync(join(dir, 'server.cfg'), `endpoint_add_tcp "0.0.0.0:${gamePort}"\nendpoint_add_udp "0.0.0.0:${gamePort}"\nsv_hostname "${name}"\n`)
      return dir
    },
  }
  const stop = async () => {
    if (exited === null) {
      child.kill('SIGTERM')
      await new Promise((r) => setTimeout(r, 500))
      if (exited === null) child.kill('SIGKILL')
    }
    rmSync(root, { recursive: true, force: true })
  }
  return { panel, stop }
}

/** Errors and failures a page produced; the test fails at the end if there were any (unless expected). */
class Trouble {
  readonly list: string[] = []
  private allowed: RegExp[] = []
  expect(re: RegExp) {
    this.allowed.push(re)
  }
  add(t: string) {
    if (!this.allowed.some((re) => re.test(t))) this.list.push(t)
  }
}

export const test = base.extend<{ trouble: Trouble; signedIn: Page }, { panel: Panel }>({
  panel: [
    // Playwright needs the destructuring pattern even when there is nothing to take from it.
    // eslint-disable-next-line no-empty-pattern
    async ({}, use) => {
      const { panel, stop } = await startPanel()
      try {
        await use(panel)
      } finally {
        await stop()
      }
    },
    { scope: 'worker', timeout: 90_000 },
  ],
  trouble: async ({ page, panel }, use) => {
    const t = new Trouble()
    page.on('console', (m) => m.type() === 'error' && t.add(`console.error: ${m.text()}`))
    page.on('pageerror', (e) => t.add(`page error: ${e.message}`))
    // A request the browser itself cancelled (the page moved on while it was in flight) is not a failure.
    page.on('requestfailed', (r) => r.failure()?.errorText !== 'net::ERR_ABORTED' && t.add(`request failed: ${r.method()} ${r.url()} (${r.failure()?.errorText})`))
    page.on('response', (r) => {
      // Our own panel only: a 4xx/5xx from it is a failed request, whatever the page does with it.
      if (r.url().startsWith(panel.origin) && r.status() >= 400) t.add(`HTTP ${r.status()} ${r.request().method()} ${r.url()}`)
    })
    await use(t)
    expect(t.list, `the browser console and requests must stay clean.\n--- panel output ---\n${panel.log().slice(-1500)}`).toEqual([])
  },
  // A page already signed in through the printed one-time link.
  signedIn: async ({ page, panel, trouble: _t }, use) => {
    await page.goto(await panel.newLink())
    await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible({ timeout: 30_000 })
    await use(page)
  },
})

export { expect }
